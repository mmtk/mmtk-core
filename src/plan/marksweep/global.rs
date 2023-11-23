use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::marksweep::gc_work::MSGCWorkContext;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use mmtk_macros::{HasSpaces, PlanTraceObject};

#[cfg(feature = "malloc_mark_sweep")]
pub type MarkSweepSpace<VM> = crate::policy::marksweepspace::malloc_ms::MallocSpace<VM>;
#[cfg(feature = "malloc_mark_sweep")]
use crate::policy::marksweepspace::malloc_ms::MAX_OBJECT_SIZE;

#[cfg(not(feature = "malloc_mark_sweep"))]
pub type MarkSweepSpace<VM> = crate::policy::marksweepspace::native_ms::MarkSweepSpace<VM>;
#[cfg(not(feature = "malloc_mark_sweep"))]
use crate::policy::marksweepspace::native_ms::MAX_OBJECT_SIZE;

#[derive(HasSpaces, PlanTraceObject)]
pub struct MarkSweep<VM: VMBinding> {
    #[parent]
    common: CommonPlan<VM>,
    #[space]
    ms: MarkSweepSpace<VM>,
}

/// The plan constraints for the mark sweep plan.
pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    max_non_los_default_alloc_bytes: MAX_OBJECT_SIZE,
    may_trace_duplicate_edges: true,
    needs_prepare_mutator: !cfg!(feature = "malloc_mark_sweep")
        && !cfg!(feature = "eager_sweeping"),
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        scheduler.schedule_common_work::<MSGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.ms.prepare();
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.ms.release();
        self.common.release(tls, true);
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_used_pages(&self) -> usize {
        self.common.get_used_pages() + self.ms.reserved_pages()
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

    fn constraints(&self) -> &'static PlanConstraints {
        &MS_CONSTRAINTS
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let mut global_side_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        MarkSweepSpace::<VM>::extend_global_side_metadata_specs(&mut global_side_metadata_specs);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &MS_CONSTRAINTS,
            global_side_metadata_specs,
        };

        let res = MarkSweep {
            ms: MarkSweepSpace::new(plan_args.get_space_args(
                "ms",
                true,
                VMRequest::discontiguous(),
            )),
            common: CommonPlan::new(plan_args),
        };

        res.verify_side_metadata_sanity();

        res
    }

    pub fn ms_space(&self) -> &MarkSweepSpace<VM> {
        &self.ms
    }
}
