use crate::mmtk::MMTK;
use crate::plan::freelistmarksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::{BasePlan, CommonPlan, NoCopy};
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::plan::{AllocationSemantics, GcStatus};
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::{CommonSpace, Space, SpaceOptions};
use crate::scheduler::gc_work::{EndOfGC, Prepare, Release, StopMutators};
use crate::scheduler::GCWorkerLocalPtr;
use crate::scheduler::MMTkScheduler;
use crate::scheduler::{GCWorkerLocal, WorkBucketStage};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{
    SideMetadataContext, SideMetadataSanity, SideMetadataSpec, LOCAL_SIDE_METADATA_BASE_ADDRESS,
};
use crate::util::opaque_pointer::*;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::ScheduleSanityGC;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;

use super::gc_work::FLMSProcessEdges;

pub struct FreeListMarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
    pub ms_space: MarkSweepSpace<VM>,
}

pub const FLMS_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

impl<VM: VMBinding> Plan for FreeListMarkSweep<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &FLMS_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: VMWorkerThread,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = NoCopy::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);

        // FIXME correctly initialize spaces based on options
        self.ms_space.init(&vm_map);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.common.base.collection_required(self, space_full, space)
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.ms_space.reset();
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.ms_space.block_level_sweep();
        // self.ms_space.eager_sweep(tls);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<FLMSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, NoCopy<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, NoCopy<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, NoCopy<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_pages_used(&self) -> usize {
        self.common.get_pages_used() + self.ms_space.reserved_pages()
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> FreeListMarkSweep<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: Arc<MMTkScheduler<VM>>,
    ) -> Self {
        #[cfg(not(feature = "freelistmarksweep_lock_free"))]
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        #[cfg(feature = "freelistmarksweep_lock_free")]
        let heap = HeapMeta::new(HEAP_START, HEAP_END);
        let ms_space = MarkSweepSpace::new(
            "MSspace",
            true,
            VMRequest::discontiguous(),
            // local_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
            scheduler,
        );
        let global_specs = SideMetadataContext::new_global_specs(&[]);

        // let im_space = ImmortalSpace::new(
        //     "IMspace",
        //     true,
        //     VMRequest::discontiguous(),
        //     global_specs.clone(),
        //     vm_map,
        //     mmapper,
        //     &mut heap,
        //     &FLMS_CONSTRAINTS,
        // );

        let common =             CommonPlan::new(
            vm_map,
            mmapper,
            options,
            heap,
            &FLMS_CONSTRAINTS,
            global_specs,
        );

        let res = FreeListMarkSweep {
            common,
            ms_space,
        };

        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.common
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.ms_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        // res.im_space
        //     .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res
    }

    pub fn ms_space(&self) -> &MarkSweepSpace<VM> {
        &self.ms_space
    }
}
