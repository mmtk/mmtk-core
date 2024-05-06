use super::gc_work::MarkCompactGCWorkContext;
use super::gc_work::{
    CalculateForwardingAddress, Compact, ForwardingProcessEdges, MarkingProcessEdges,
    UpdateReferences,
};
use crate::plan::global::CommonPlan;
use crate::plan::global::{BasePlan, CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::markcompact::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::CopySemantics;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
#[cfg(not(feature = "vo_bit"))]
use crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;

use enum_map::EnumMap;

use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(HasSpaces, PlanTraceObject)]
pub struct MarkCompact<VM: VMBinding> {
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub mc_space: MarkCompactSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
}

/// The plan constraints for the mark compact plan.
pub const MARKCOMPACT_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    needs_forward_after_liveness: true,
    max_non_los_default_alloc_bytes:
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkCompact<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &MARKCOMPACT_CONSTRAINTS
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn prepare(&mut self, _tls: VMWorkerThread) {
        self.common.prepare(_tls, true);
        self.mc_space.prepare();
    }

    fn release(&mut self, _tls: VMWorkerThread) {
        self.common.release(_tls, true);
        self.mc_space.release();
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        // TODO use schedule_common once it can work with markcompact
        // self.common()
        //     .schedule_common::<Self, MarkingProcessEdges<VM>, NoCopy<VM>>(
        //         self,
        //         &MARKCOMPACT_CONSTRAINTS,
        //         scheduler,
        //     );

        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MarkCompactGCWorkContext<VM>>::new());

        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<MarkCompactGCWorkContext<VM>>::new(self));

        scheduler.work_buckets[WorkBucketStage::CalculateForwarding]
            .add(CalculateForwardingAddress::<VM>::new(&self.mc_space));
        // do another trace to update references
        scheduler.work_buckets[WorkBucketStage::SecondRoots].add(UpdateReferences::<VM>::new(self));
        scheduler.work_buckets[WorkBucketStage::Compact].add(Compact::<VM>::new(&self.mc_space));

        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<MarkCompactGCWorkContext<VM>>::new(self));

        // Reference processing
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
            };
            scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<MarkingProcessEdges<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());

            use crate::util::reference_processor::RefForwarding;
            scheduler.work_buckets[WorkBucketStage::RefForwarding]
                .add(RefForwarding::<ForwardingProcessEdges<VM>>::new());

            use crate::util::reference_processor::RefEnqueue;
            scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }

        // Finalization
        if !*self.base().options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            // treat finalizable objects as roots and perform a closure (marking)
            // must be done before calculating forwarding pointers
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<MarkingProcessEdges<VM>>::new());
            // update finalizable object references
            // must be done before compacting
            scheduler.work_buckets[WorkBucketStage::FinalizableForwarding]
                .add(ForwardFinalization::<ForwardingProcessEdges<VM>>::new());
        }

        // VM-specific weak ref processing
        scheduler.work_buckets[WorkBucketStage::VMRefClosure]
            .set_sentinel(Box::new(VMProcessWeakRefs::<MarkingProcessEdges<VM>>::new()));

        // VM-specific weak ref forwarding
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding]
            .add(VMForwardWeakRefs::<ForwardingProcessEdges<VM>>::new());

        // VM-specific work after forwarding, possible to implement ref enququing.
        scheduler.work_buckets[WorkBucketStage::Release].add(VMPostForwarding::<VM>::default());

        // Analysis GC work
        #[cfg(feature = "analysis")]
        {
            use crate::util::analysis::GcHookWork;
            scheduler.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
        }
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(crate::util::sanity::sanity_checker::ScheduleSanityGC::<Self>::new(self));
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_used_pages(&self) -> usize {
        self.mc_space.reserved_pages() + self.common.get_used_pages()
    }

    fn get_collection_reserved_pages(&self) -> usize {
        0
    }
}

impl<VM: VMBinding> MarkCompact<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        // if vo_bit is enabled, VO_BIT_SIDE_METADATA_SPEC will be added to
        // SideMetadataContext by default, so we don't need to add it here.
        #[cfg(feature = "vo_bit")]
        let global_side_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        // if vo_bit is NOT enabled,
        // we need to add VO_BIT_SIDE_METADATA_SPEC to SideMetadataContext here.
        #[cfg(not(feature = "vo_bit"))]
        let global_side_metadata_specs =
            SideMetadataContext::new_global_specs(&[VO_BIT_SIDE_METADATA_SPEC]);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &MARKCOMPACT_CONSTRAINTS,
            global_side_metadata_specs,
        };

        let mc_space =
            MarkCompactSpace::new(plan_args.get_space_args("mc", true, VMRequest::discontiguous()));

        let res = MarkCompact {
            mc_space,
            common: CommonPlan::new(plan_args),
        };

        res.verify_side_metadata_sanity();

        res
    }
}

impl<VM: VMBinding> MarkCompact<VM> {
    pub fn mc_space(&self) -> &MarkCompactSpace<VM> {
        &self.mc_space
    }
}
