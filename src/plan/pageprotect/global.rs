use super::gc_work::PPGCWorkContext;
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::{plan::global::BasePlan, vm::VMBinding};
use crate::{
    plan::global::CommonPlan, policy::largeobjectspace::LargeObjectSpace,
    util::opaque_pointer::VMWorkerThread,
};
use enum_map::EnumMap;

use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(HasSpaces, PlanTraceObject)]
pub struct PageProtect<VM: VMBinding> {
    #[space]
    pub space: LargeObjectSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
}

/// The plan constraints for the page protect plan.
pub const CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for PageProtect<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &CONSTRAINTS
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        scheduler.schedule_common_work::<PPGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.space.prepare(true);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.space.release(true);
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_used_pages(&self) -> usize {
        self.space.reserved_pages() + self.common.get_used_pages()
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

impl<VM: VMBinding> PageProtect<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
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

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&[]),
        };

        let ret = PageProtect {
            space: LargeObjectSpace::new(
                plan_args.get_space_args("pageprotect", true, VMRequest::discontiguous()),
                true,
            ),
            common: CommonPlan::new(plan_args),
        };

        ret.verify_side_metadata_sanity();

        ret
    }
}
