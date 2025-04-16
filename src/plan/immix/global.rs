use super::barrier::ImmixFakeFieldBarrierSemantics;
use super::gc_work::ImmixGCWorkContext;
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::block::Block;
use crate::policy::immix::ImmixSpaceArgs;
use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST};
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::MetadataSpec;
use crate::vm::VMBinding;
use crate::BarrierSelector;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use std::sync::atomic::AtomicBool;

use atomic::Ordering;
use enum_map::EnumMap;

use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(HasSpaces, PlanTraceObject)]
pub struct Immix<VM: VMBinding> {
    #[post_scan]
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub immix_space: ImmixSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
    last_gc_was_defrag: AtomicBool,
}

/// The plan constraints for the immix plan.
pub const IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: crate::policy::immix::DEFRAG,
    // Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    needs_log_bit: crate::args::BARRIER_MEASUREMENT,
    needs_field_log_bit: crate::args::BARRIER_MEASUREMENT,
    barrier: if crate::args::BARRIER_MEASUREMENT
        && !cfg!(feature = "barrier_measurement_no_barrier")
    {
        BarrierSelector::FieldBarrier
    } else {
        BarrierSelector::NoBarrier
    },
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for Immix<VM> {
    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        ImmixSpace::<VM>::is_last_gc_exhaustive(self.last_gc_was_defrag.load(Ordering::Relaxed))
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &IMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix_space)],
            constraints: &IMMIX_CONSTRAINTS,
        }
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.disable_unnecessary_buckets(scheduler);
        Self::schedule_immix_full_heap_collection::<
            Immix<VM>,
            ImmixGCWorkContext<VM, TRACE_KIND_FAST>,
            ImmixGCWorkContext<VM, TRACE_KIND_DEFRAG>,
        >(self, &self.immix_space, scheduler)
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.immix_space.prepare(
            true,
            false,
            crate::policy::immix::defrag::StatsForDefrag::new(self),
        );
    }

    fn no_worker_prepare(&self) -> bool {
        true
    }

    fn fast_worker_release(&self) -> bool {
        true
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.immix_space.release(true);
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {
        self.last_gc_was_defrag
            .store(self.immix_space.end_of_gc(), Ordering::Relaxed);
    }

    fn current_gc_may_move_object(&self) -> bool {
        self.immix_space.in_defrag()
    }

    fn gc_pause_start(&self, _scheduler: &GCWorkScheduler<VM>) {
        Block::update_global_phase_epoch(&self.immix_space);
    }

    fn gc_pause_end(&self) {
        Block::update_global_phase_epoch(&self.immix_space);
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.immix_space.reserved_pages() + self.common.get_used_pages()
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
}

impl<VM: VMBinding> Immix<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let immix_specs =
            metadata::extract_side_metadata(&[MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE)]);
        crate::args::validate_features(IMMIX_CONSTRAINTS.barrier, &args.options);
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &IMMIX_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&immix_specs),
        };
        if crate::args::BARRIER_MEASUREMENT {
            plan_args
                .global_side_metadata_specs
                .push(ImmixFakeFieldBarrierSemantics::<VM>::UNLOG_BITS);
            plan_args
                .global_side_metadata_specs
                .push(ImmixFakeFieldBarrierSemantics::<VM>::LOCK_BITS);
        }
        Self::new_with_args(
            plan_args,
            ImmixSpaceArgs {
                reset_log_bit_in_major_gc: false,
                unlog_object_when_traced: false,
                #[cfg(feature = "vo_bit")]
                mixed_age: false,
            },
        )
    }

    pub fn new_with_args(
        mut plan_args: CreateSpecificPlanArgs<VM>,
        space_args: ImmixSpaceArgs,
    ) -> Self {
        let immix = Immix {
            immix_space: ImmixSpace::new(
                plan_args.get_space_args("immix", true, false, VMRequest::discontiguous()),
                space_args,
            ),
            common: CommonPlan::new(plan_args),
            last_gc_was_defrag: AtomicBool::new(false),
        };

        immix.verify_side_metadata_sanity();

        immix
    }

    /// Schedule a full heap immix collection. This method is used by immix/genimmix/stickyimmix
    /// to schedule a full heap collection. A plan must call set_collection_kind and set_gc_status before this method.
    pub(crate) fn schedule_immix_full_heap_collection<
        PlanType: Plan<VM = VM>,
        FastContext: GCWorkContext<VM = VM, PlanType = PlanType>,
        DefragContext: GCWorkContext<VM = VM, PlanType = PlanType>,
    >(
        plan: &'static DefragContext::PlanType,
        immix_space: &ImmixSpace<VM>,
        scheduler: &GCWorkScheduler<VM>,
    ) {
        let in_defrag = immix_space.decide_whether_to_defrag(
            plan.base().global_state.is_emergency_collection(),
            true,
            plan.base()
                .global_state
                .cur_collection_attempts
                .load(Ordering::SeqCst),
            plan.base().global_state.is_user_triggered_collection(),
            *plan.base().options.full_heap_system_gc,
        );

        if in_defrag {
            scheduler.schedule_common_work::<DefragContext>(plan);
        } else {
            scheduler.schedule_common_work::<FastContext>(plan);
        }
    }

    pub(in crate::plan) fn set_last_gc_was_defrag(&self, defrag: bool, order: Ordering) {
        self.last_gc_was_defrag.store(defrag, order)
    }

    fn disable_unnecessary_buckets(&self, scheduler: &GCWorkScheduler<VM>) {
        scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::Initial].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::TPinningClosure].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::PinningRootsTrace].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::VMRefClosure].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::SecondRoots].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::RefForwarding].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::FinalizableForwarding].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::Compact].set_as_disabled();
        scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].set_as_disabled();
    }
}
