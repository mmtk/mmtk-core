use crate::plan::global::BasePlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::EnumMap;

#[cfg(not(feature = "nogc_lock_free"))]
use crate::policy::immortalspace::ImmortalSpace as NoGCImmortalSpace;
#[cfg(feature = "nogc_lock_free")]
use crate::policy::lockfreeimmortalspace::LockFreeImmortalSpace as NoGCImmortalSpace;

pub struct NoGC<VM: VMBinding> {
    pub base: BasePlan<VM>,
    pub nogc_space: NoGCImmortalSpace<VM>,
    pub immortal: ImmortalSpace<VM>,
    pub los: ImmortalSpace<VM>,
}

pub const NOGC_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

impl<VM: VMBinding> Plan for NoGC<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &NOGC_CONSTRAINTS
    }

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.base.get_spaces();
        ret.push(&self.nogc_space);
        ret.push(&self.immortal);
        ret.push(&self.los);
        ret
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.base
    }

    fn prepare(&mut self, _tls: VMWorkerThread) {
        unreachable!()
    }

    fn release(&mut self, _tls: VMWorkerThread) {
        unreachable!()
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, _scheduler: &GCWorkScheduler<VM>) {
        unreachable!("GC triggered in nogc")
    }

    fn get_used_pages(&self) -> usize {
        self.nogc_space.reserved_pages()
            + self.immortal.reserved_pages()
            + self.los.reserved_pages()
            + self.base.get_used_pages()
    }

    fn handle_user_collection_request(
        &self,
        _tls: VMMutatorThread,
        _force: bool,
        _exhaustive: bool,
    ) {
        warn!("User attempted a collection request, but it is not supported in NoGC. The request is ignored.");
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &NOGC_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&[]),
        };

        let res = NoGC {
            nogc_space: NoGCImmortalSpace::new(plan_args.get_space_args(
                "nogc_space",
                cfg!(not(feature = "nogc_no_zeroing")),
                VMRequest::discontiguous(),
            )),
            immortal: ImmortalSpace::new(plan_args.get_space_args(
                "immortal",
                true,
                VMRequest::discontiguous(),
            )),
            los: ImmortalSpace::new(plan_args.get_space_args(
                "los",
                true,
                VMRequest::discontiguous(),
            )),
            base: BasePlan::new(plan_args),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.base()
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.nogc_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);

        res
    }
}
