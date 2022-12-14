use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::global::GcStatus;
use crate::plan::marksweep::gc_work::MSGCWorkContext;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use mmtk_macros::PlanTraceObject;

#[cfg(feature = "malloc_mark_sweep")]
pub type MarkSweepSpace<VM> = crate::policy::marksweepspace::malloc_ms::MallocSpace<VM>;
#[cfg(feature = "malloc_mark_sweep")]
use crate::policy::marksweepspace::malloc_ms::MAX_OBJECT_SIZE;

#[cfg(not(feature = "malloc_mark_sweep"))]
pub type MarkSweepSpace<VM> = crate::policy::marksweepspace::native_ms::MarkSweepSpace<VM>;
#[cfg(not(feature = "malloc_mark_sweep"))]
use crate::policy::marksweepspace::native_ms::MAX_OBJECT_SIZE;

#[derive(PlanTraceObject)]
pub struct MarkSweep<VM: VMBinding> {
    #[fallback_trace]
    common: CommonPlan<VM>,
    #[trace]
    ms: MarkSweepSpace<VM>,
}

pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    max_non_los_default_alloc_bytes: MAX_OBJECT_SIZE,
    may_trace_duplicate_edges: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.common.get_spaces();
        ret.push(&self.ms);
        ret
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.schedule_common_work::<MSGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
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

        let mut common_plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &MS_CONSTRAINTS,
            global_side_metadata_specs,
        };

        let res = MarkSweep {
            ms: MarkSweepSpace::new(common_plan_args.get_space_args(
                "ms",
                true,
                VMRequest::discontiguous(),
            )),
            common: CommonPlan::new(common_plan_args),
        };

        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.common
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.ms
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res
    }

    pub fn ms_space(&self) -> &MarkSweepSpace<VM> {
        &self.ms
    }
}
