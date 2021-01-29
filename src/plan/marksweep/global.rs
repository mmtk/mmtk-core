use super::gc_works::MSProcessEdges;
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan;
#[cfg(all(feature = "largeobjectspace", feature = "immortalspace"))]
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::global::NoCopy;
use crate::plan::marksweep::malloc::ms_free;
use crate::plan::marksweep::malloc::ms_malloc_usable_size;
use crate::plan::marksweep::metadata::unset_alloc_bit;
use crate::plan::marksweep::metadata::unset_mark_bit;
use crate::plan::marksweep::metadata::ALLOCATION_METADATA_ID;
use crate::plan::marksweep::metadata::MAPPED_CHUNKS;
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

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = NoCopy<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let heap = HeapMeta::new(HEAP_START, HEAP_END);
        MarkSweep {
            base: BasePlan::new(vm_map, mmapper, options, heap),
            space: MallocSpace::new(),
        }
    }

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
            ALLOCATION_METADATA_ID = SideMetadata::request_meta_bits(1, align);
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
        scheduler.work_buckets[WorkBucketStage::Prepare].add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release].add(Release::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC);
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        _mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_ms_mutator(tls, self))
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, _tls: OpaquePointer) {
        // Do nothing
    }

    fn release(&self, _tls: OpaquePointer) {
        let mut now_empty = HashSet::new();
        unsafe {
            for chunk_start in &*MAPPED_CHUNKS.read().unwrap() {
                let mut empty_chunk = true;
                let mut address = *chunk_start;
                let chunk_end = chunk_start.add(BYTES_IN_CHUNK);
                while address.as_usize() < chunk_end.as_usize() {
                    if SideMetadata::load_atomic(ALLOCATION_METADATA_ID, address) == 1 {
                        if SideMetadata::load_atomic(MARKING_METADATA_ID, address) == 0 {
                            let ptr = address.to_mut_ptr();
                            HEAP_USED.fetch_sub(ms_malloc_usable_size(ptr), Ordering::SeqCst);
                            ms_free(ptr);
                            unset_alloc_bit(address);
                        } else {
                            unset_mark_bit(address);
                            empty_chunk = false;
                        }
                    }
                    address = address.add(VM::MAX_ALIGNMENT);
                }
                if empty_chunk {
                    now_empty.insert(chunk_start.as_usize());
                }
            }
            MAPPED_CHUNKS
                .write()
                .unwrap()
                .retain(|c| !now_empty.contains(&c.as_usize()));
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

    #[cfg(all(feature = "largeobjectspace", feature = "immortalspace"))]
    fn common(&self) -> &CommonPlan<VM> {
        unreachable!("MarkSweep does not have a common plan.");
    }
}
