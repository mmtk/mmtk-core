use super::mutator::ALLOCATOR_MAPPING;
use super::{
    gc_work::{GenCopyCopyContext, GenCopyMatureProcessEdges, GenCopyNurseryProcessEdges},
    LOGGING_META,
};
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::side_metadata::meta_bytes_per_chunk;
use crate::util::OpaquePointer;
use crate::vm::ObjectModel;
use crate::vm::*;
use crate::{mmtk::MMTK, plan::barriers::BarrierSelector};
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const ALLOC_SS: AllocationSemantics = AllocationSemantics::Default;
pub const NURSERY_SIZE: usize = 32 * 1024 * 1024;

pub struct GenCopy<VM: VMBinding> {
    pub nursery: CopySpace<VM>,
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
    in_nursery: AtomicBool,
    pub scheduler: &'static MMTkScheduler<VM>,
}

unsafe impl<VM: VMBinding> Sync for GenCopy<VM> {}

pub const GENCOPY_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    barrier: super::ACTIVE_BARRIER,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for GenCopy<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &GENCOPY_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = GenCopyCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        let nursery_full = self.nursery.reserved_pages() >= (NURSERY_SIZE >> LOG_BYTES_IN_PAGE);
        let heap_full = self.get_pages_reserved() > self.get_total_pages();
        space_full || nursery_full || heap_full
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.nursery.init(&vm_map);
        self.copyspace0.init(&vm_map);
        self.copyspace1.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        let in_nursery = !self.request_full_heap_collection();
        self.in_nursery.store(in_nursery, Ordering::SeqCst);
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        if in_nursery {
            self.common()
                .schedule_common::<GenCopyNurseryProcessEdges<VM>>(&GENCOPY_CONSTRAINTS, scheduler);
        } else {
            self.common()
                .schedule_common::<GenCopyMatureProcessEdges<VM>>(&GENCOPY_CONSTRAINTS, scheduler);
        }

        // Stop & scan mutators (mutator scanning can happen before STW)
        if in_nursery {
            scheduler.work_buckets[WorkBucketStage::Unconstrained]
                .add(StopMutators::<GenCopyNurseryProcessEdges<VM>>::new());
        } else {
            scheduler.work_buckets[WorkBucketStage::Unconstrained]
                .add(StopMutators::<GenCopyMatureProcessEdges<VM>>::new());
        }
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, GenCopyCopyContext<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, GenCopyCopyContext<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, GenCopyCopyContext<VM>>::new());
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer, _mmtk: &'static MMTK<VM>) {
        self.common.prepare(tls, true);
        self.nursery.prepare(true);
        if !self.in_nursery() {
            self.hi
                .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
        }
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    fn release(&self, tls: OpaquePointer, _mmtk: &'static MMTK<VM>) {
        self.common.release(tls, true);
        self.nursery.release();
        if !self.in_nursery() {
            self.fromspace().release();
        }
    }

    fn get_collection_reserve(&self) -> usize {
        self.nursery.reserved_pages() + self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages()
            + self.tospace().reserved_pages()
            + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn in_nursery(&self) -> bool {
        self.in_nursery.load(Ordering::SeqCst)
    }

    fn global_side_metadata_per_chunk(&self) -> usize {
        let mut side_metadata_per_chunk = if !VM::VMObjectModel::HAS_GC_BYTE {
            meta_bytes_per_chunk(3, 1)
        } else {
            0
        };
        if super::ACTIVE_BARRIER == BarrierSelector::ObjectBarrier {
            side_metadata_per_chunk += LOGGING_META.meta_bytes_per_chunk();
        }
        side_metadata_per_chunk
    }
}

impl<VM: VMBinding> GenCopy<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: &'static MMTkScheduler<VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        GenCopy {
            nursery: CopySpace::new(
                "nursery",
                false,
                true,
                VMRequest::fixed_extent(NURSERY_SIZE, false),
                vm_map,
                mmapper,
                &mut heap,
            ),
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap, &GENCOPY_CONSTRAINTS),
            in_nursery: AtomicBool::default(),
            scheduler,
        }
    }

    fn request_full_heap_collection(&self) -> bool {
        // For barrier overhead measurements, we always do full gc in nursery collections.
        if super::FULL_NURSERY_GC {
            return true;
        }
        self.get_total_pages() <= self.get_pages_reserved()
    }

    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }
}
