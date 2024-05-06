use super::gc_work::SSGCWorkContext;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::semispace::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::{plan::global::BasePlan, vm::VMBinding};
use std::sync::atomic::{AtomicBool, Ordering};

use mmtk_macros::{HasSpaces, PlanTraceObject};

use enum_map::EnumMap;

#[derive(HasSpaces, PlanTraceObject)]
pub struct SemiSpace<VM: VMBinding> {
    pub hi: AtomicBool,
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub copyspace0: CopySpace<VM>,
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub copyspace1: CopySpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
}

/// The plan constraints for the semispace plan.
pub const SS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    max_non_los_default_alloc_bytes:
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for SemiSpace<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &SS_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::CopySpace(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![
                // // The tospace argument doesn't matter, we will rebind before a GC anyway.
                (CopySelector::CopySpace(0), &self.copyspace0),
            ],
            constraints: &SS_CONSTRAINTS,
        }
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        scheduler.schedule_common_work::<SSGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
                                                                       // prepare each of the collected regions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
        self.fromspace_mut()
            .set_copy_for_sft_trace(Some(CopySemantics::DefaultCopy));
        self.tospace_mut().set_copy_for_sft_trace(None);
    }

    fn prepare_worker(&self, worker: &mut GCWorker<VM>) {
        unsafe { worker.get_copy_context_mut().copy[0].assume_init_mut() }.rebind(self.tospace());
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.fromspace().release();
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.tospace().reserved_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_used_pages()
    }

    fn get_available_pages(&self) -> usize {
        (self
            .get_total_pages()
            .saturating_sub(self.get_reserved_pages()))
            >> 1
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

impl<VM: VMBinding> SemiSpace<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &SS_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&[]),
        };

        let res = SemiSpace {
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                plan_args.get_space_args("copyspace0", true, VMRequest::discontiguous()),
                false,
            ),
            copyspace1: CopySpace::new(
                plan_args.get_space_args("copyspace1", true, VMRequest::discontiguous()),
                true,
            ),
            common: CommonPlan::new(plan_args),
        };

        res.verify_side_metadata_sanity();

        res
    }

    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn tospace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace1
        } else {
            &mut self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn fromspace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace0
        } else {
            &mut self.copyspace1
        }
    }
}
