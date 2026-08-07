use super::gc_work::OVCWorkContext;
use super::gc_work::{AfterCompact, ForwardingTrace, GenerateWork, MarkingTrace, UpdateReferences};
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::global::{BasePlan, CommonPlan};
use crate::plan::markcompact::ovc::mutator::ALLOCATOR_MAPPING;
use crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN;
use crate::plan::tracing::gc_work::weakref::{
    VMForwardWeakRefs, VMPostForwarding, VMProcessWeakRefs,
};
use crate::plan::{AllocationSemantics, Plan, PlanConstraints};
use crate::policy::ovc::OVCSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::{GCWorkScheduler, WorkBucketStage};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::gc_trigger::SpaceStats;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use mmtk_macros::{HasSpaces, PlanTraceObject};

/// [`OVC`] implements a stop-the-world and parallel implementation of
/// the Compressor, as described in Kermany and Petrank,
/// [The Compressor: concurrent, incremental, and parallel compaction](https://dl.acm.org/doi/10.1145/1133255.1134023).
#[derive(HasSpaces, PlanTraceObject)]
pub struct OVC<VM: VMBinding> {
    #[parent]
    pub common: CommonPlan<VM>,
    #[space]
    pub ovc_space: OVCSpace<VM>,
}

/// The plan constraints for the OVC plan.
pub const OVC_CONSTRAINTS: PlanConstraints = PlanConstraints {
    max_non_los_default_alloc_bytes: MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    moves_objects: true,
    needs_forward_after_liveness: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for OVC<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &OVC_CONSTRAINTS
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn common_mut(&mut self) -> &mut CommonPlan<VM> {
        &mut self.common
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.common.base
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.ovc_space.prepare();
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.ovc_space.release();
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        // TODO: Extract a method similar to `GCWorkScheduler::schedule_common_work`
        // but for compressing plans.

        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<OVCWorkContext<VM>>::new());

        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<OVCWorkContext<VM>>::new(self));

        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].add(GenerateWork::new(
            &self.ovc_space,
            OVCSpace::<VM>::add_offset_vector_tasks,
        ));

        // scan roots to update their references
        scheduler.work_buckets[WorkBucketStage::SecondRoots].add(UpdateReferences::<VM>::new());

        scheduler.work_buckets[WorkBucketStage::Compact].add(GenerateWork::new(
            &self.ovc_space,
            OVCSpace::<VM>::add_compact_tasks,
        ));

        scheduler.work_buckets[WorkBucketStage::Compact].set_sentinel(Box::new(
            AfterCompact::<VM>::new(&self.ovc_space, &self.common.los),
        ));

        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<OVCWorkContext<VM>>::new(self));

        // Reference processing
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
            };
            scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<MarkingTrace<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());

            use crate::util::reference_processor::RefForwarding;
            scheduler.work_buckets[WorkBucketStage::RefForwarding]
                .add(RefForwarding::<ForwardingTrace<VM>>::new());

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
                .add(Finalization::<MarkingTrace<VM>>::new());
            // update finalizable object references
            // must be done before compacting
            scheduler.work_buckets[WorkBucketStage::FinalizableForwarding]
                .add(ForwardFinalization::<ForwardingTrace<VM>>::new());
        }

        // VM-specific weak ref processing
        scheduler.work_buckets[WorkBucketStage::VMRefClosure]
            .set_sentinel(Box::new(VMProcessWeakRefs::<MarkingTrace<VM>>::new()));

        // VM-specific weak ref forwarding
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding]
            .add(VMForwardWeakRefs::<ForwardingTrace<VM>>::new());

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

    fn current_gc_may_move_object(&self) -> bool {
        true
    }

    fn get_used_pages(&self) -> usize {
        self.ovc_space.reserved_pages() + self.common.get_used_pages()
    }
}

impl<VM: VMBinding> OVC<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &OVC_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&[]),
        };

        OVC {
            ovc_space: OVCSpace::new(plan_args.get_normal_space_args(
                "ovc_space",
                true,
                false,
                VMRequest::discontiguous(),
            )),
            common: CommonPlan::new(plan_args),
        }
    }
}
