use super::gc_work::PPProcessEdges;
use super::mutator::ALLOCATOR_MAPPING;
use crate::mmtk::MMTK;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(feature = "analysis")]
use crate::util::analysis::GcHookWork;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::{plan::global::BasePlan, vm::VMBinding};
use crate::{
    plan::global::{CommonPlan, NoCopy},
    policy::largeobjectspace::LargeObjectSpace,
    util::opaque_pointer::VMWorkerThread,
};
use enum_map::EnumMap;
use std::sync::Arc;

pub struct PageProtect<VM: VMBinding> {
    pub space: LargeObjectSpace<VM>,
    pub common: CommonPlan<VM>,
}

pub const CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for PageProtect<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &CONSTRAINTS
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
        // Warn users that the plan may fail due to maximum mapping allowed.
        warn!(
            "PageProtect uses a high volume of memory mappings. \
            If you encounter failures in memory protect/unprotect in this plan,\
            consider increase the maximum mapping allowed by the OS{}.",
            if cfg!(target_os = "linux") {
                " (e.g. sudo sysctl -w vm.max_map_count=655300)"
            } else {
                ""
            }
        );
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.space.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        self.common()
            .schedule_common::<PPProcessEdges<VM>>(&CONSTRAINTS, scheduler);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<PPProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, NoCopy<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, NoCopy<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::RefClosure]
            .add(ProcessWeakRefs::<PPProcessEdges<VM>>::new());
        // Scheduling all the gc hooks of analysis routines. It is generally recommended
        // to take advantage of the scheduling system we have in place for more performance
        #[cfg(feature = "analysis")]
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, NoCopy<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.space.prepare(true);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.space.release(true);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_pages_used(&self) -> usize {
        self.space.reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> PageProtect<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);

        PageProtect {
            space: LargeObjectSpace::new(
                "los",
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
                &CONSTRAINTS,
                false,
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &CONSTRAINTS,
                global_metadata_specs,
            ),
        }
    }
}
