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
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::util::rust_util::flex_mut::ArcFlexMut;
use crate::{plan::global::BasePlan, vm::VMBinding};
use std::sync::atomic::{AtomicBool, Ordering};

use mmtk_macros::{HasSpaces, PlanTraceObject};

use enum_map::EnumMap;

#[derive(HasSpaces, PlanTraceObject)]
pub struct SemiSpace<VM: VMBinding> {
    pub hi: AtomicBool,
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub copyspace0: ArcFlexMut<CopySpace<VM>>,
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub copyspace1: ArcFlexMut<CopySpace<VM>>,
    #[parent]
    pub common: CommonPlan<VM>,
}

pub const SS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    max_non_los_default_alloc_bytes:
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
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
                (
                    CopySelector::CopySpace(0),
                    self.copyspace0.clone().into_dyn_space(),
                ),
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

    fn prepare(&self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
                                                                       // prepare each of the collected regions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.read().prepare(hi);
        self.copyspace1.read().prepare(!hi);
        self.fromspace()
            .write()
            .set_copy_for_sft_trace(Some(CopySemantics::DefaultCopy));
        self.tospace().write().set_copy_for_sft_trace(None);
    }

    fn prepare_worker(&self, worker: &mut GCWorker<VM>) {
        unsafe { worker.get_copy_context_mut().copy[0].assume_init_mut() }
            .rebind(self.tospace().clone());
    }

    fn release(&self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.fromspace().read().release();
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.tospace().read().reserved_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.tospace().read().reserved_pages() + self.common.get_used_pages()
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
            copyspace0: ArcFlexMut::new(CopySpace::new(
                plan_args.get_space_args("copyspace0", true, VMRequest::discontiguous()),
                false,
            )),
            copyspace1: ArcFlexMut::new(CopySpace::new(
                plan_args.get_space_args("copyspace1", true, VMRequest::discontiguous()),
                true,
            )),
            common: CommonPlan::new(plan_args),
        };

        res.verify_side_metadata_sanity();

        res
    }

    pub fn tospace(&self) -> &ArcFlexMut<CopySpace<VM>> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &ArcFlexMut<CopySpace<VM>> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }
}
