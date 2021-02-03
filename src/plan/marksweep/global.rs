use super::gc_works::MSProcessEdges;
use crate::{mmtk::MMTK, plan::PlanConstraints};
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::global::NoCopy;
use crate::plan::marksweep::malloc::ms_free;
use crate::plan::marksweep::malloc::ms_malloc_usable_size;
use crate::plan::marksweep::metadata::unset_alloc_bit;
use crate::plan::marksweep::metadata::unset_mark_bit;
use crate::plan::marksweep::metadata::ALLOC_METADATA_ID;
use crate::plan::marksweep::metadata::ACTIVE_CHUNKS;
use crate::plan::marksweep::metadata::MARKING_METADATA_ID;
use crate::plan::marksweep::mutator::create_ms_mutator;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::mutator_context::Mutator;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::malloc_allocator::HEAP_SIZE;
use crate::util::alloc::malloc_allocator::HEAP_USED;
use crate::util::constants;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::side_metadata::SideMetadata;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::{collections::HashSet, sync::Arc};

use atomic::Ordering;
use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

pub struct MarkSweep<VM: VMBinding> {
    pub base: BasePlan<VM>,
    pub space: MallocSpace<VM>,
}

unsafe impl<VM: VMBinding> Sync for MarkSweep<VM> {}

pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;

    fn collection_required(&self, _space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        unsafe { HEAP_USED.load(Ordering::SeqCst) >= HEAP_SIZE }
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        unsafe {
            let align = constants::LOG_BYTES_IN_WORD as usize;
            HEAP_SIZE = heap_size;
            ALLOC_METADATA_ID = SideMetadata::request_meta_bits(1, align);
            MARKING_METADATA_ID = SideMetadata::request_meta_bits(1, align);
        }
        self.base.gc_init(heap_size, vm_map, scheduler);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, NoCopy<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, NoCopy<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, NoCopy<VM>>::new());
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, _tls: OpaquePointer) {
        // Do nothing
    }

    fn release(&self, _tls: OpaquePointer) {
        let mut released_chunks = HashSet::new();
        unsafe {
            for chunk_start in &*ACTIVE_CHUNKS.read().unwrap() {
                let mut chunk_is_empty = true;
                let mut address = *chunk_start;
                let chunk_end = chunk_start.add(BYTES_IN_CHUNK);
                while address.as_usize() < chunk_end.as_usize() {
                    if SideMetadata::load_atomic(ALLOC_METADATA_ID, address) == 1 {
                        if SideMetadata::load_atomic(MARKING_METADATA_ID, address) == 0 {
                            let ptr = address.to_mut_ptr();
                            HEAP_USED.fetch_sub(ms_malloc_usable_size(ptr), Ordering::SeqCst);
                            ms_free(ptr);
                            unset_alloc_bit(address);
                        } else {
                            unset_mark_bit(address);
                            chunk_is_empty = false;
                        }
                    }
                    address = address.add(VM::MAX_ALIGNMENT);
                }
                if chunk_is_empty {
                    released_chunks.insert(chunk_start.as_usize());
                }
            }
            ACTIVE_CHUNKS
                .write()
                .unwrap()
                .retain(|c| !released_chunks.contains(&c.as_usize()));
        }
    }

    fn get_collection_reserve(&self) -> usize {
        unimplemented!();
    }

    fn get_pages_used(&self) -> usize {
        self.base.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        unreachable!("MarkSweep does not have a common plan.");
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &MS_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = NoCopy::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<VM>,
    ) -> Self {
        let heap = HeapMeta::new(HEAP_START, HEAP_END);
        MarkSweep {
            base: BasePlan::new(vm_map, mmapper, options, heap, &MS_CONSTRAINTS),
            space: MallocSpace::new(),
        }
    }
}