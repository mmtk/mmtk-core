// ANCHOR: imports_no_gc_work
use crate::plan::global::BasePlan; //Modify
use crate::plan::global::CommonPlan; // Add
use crate::plan::global::{CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::mygc::mutator::ALLOCATOR_MAPPING;
use crate::plan::mygc::gc_work::MyGCWorkContext;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace; // Add
use crate::policy::space::Space;
use crate::scheduler::*; // Modify
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering}; // Add
// ANCHOR_END: imports_no_gc_work

// Remove #[allow(unused_imports)].
// Remove handle_user_collection_request().

use mmtk_macros::{HasSpaces, PlanTraceObject};

// Modify
// ANCHOR: plan_def
#[derive(HasSpaces, PlanTraceObject)]
pub struct MyGC<VM: VMBinding> {
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
// ANCHOR_END: plan_def

// ANCHOR: constraints
pub const MYGC_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    ..PlanConstraints::default()
};
// ANCHOR_END: constraints

impl<VM: VMBinding> Plan for MyGC<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &MYGC_CONSTRAINTS
    }

    // ANCHOR: create_copy_config
    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::CopySpace(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![
                // The tospace argument doesn't matter, we will rebind before a GC anyway.
                (CopySelector::CopySpace(0), &self.copyspace0)
            ],
            constraints: &MYGC_CONSTRAINTS,
        }
    }
    // ANCHOR_END: create_copy_config

    // Modify
    // ANCHOR: schedule_collection
    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        scheduler.schedule_common_work::<MyGCWorkContext<VM>>(self);
    }
    // ANCHOR_END: schedule_collection

    // ANCHOR: collection_required()
    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }
    // ANCHOR_END: collection_required()

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    // Modify
    // ANCHOR: prepare
    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
        // Flips 'hi' to flip space definitions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);

        self.fromspace_mut()
            .set_copy_for_sft_trace(Some(CopySemantics::DefaultCopy));
        self.tospace_mut().set_copy_for_sft_trace(None);
    }
    // ANCHOR_END: prepare

    // Add
    // ANCHOR: prepare_worker
    fn prepare_worker(&self, worker: &mut GCWorker<VM>) {
        unsafe { worker.get_copy_context_mut().copy[0].assume_init_mut() }.rebind(self.tospace());
    }
    // ANCHOR_END: prepare_worker

    // Modify
    // ANCHOR: release
    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.fromspace().release();
    }
    // ANCHOR_END: release

    // Modify
    // ANCHOR: plan_get_collection_reserve
    fn get_collection_reserved_pages(&self) -> usize {
        self.tospace().reserved_pages()
    }
    // ANCHOR_END: plan_get_collection_reserve

    // Modify
    // ANCHOR: plan_get_used_pages
    fn get_used_pages(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_used_pages()
    }
    // ANCHOR_END: plan_get_used_pages

    // Modify
    // ANCHOR: plan_base
    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.common.base
    }
    // ANCHOR_END: plan_base

    // Add
    // ANCHOR: plan_common
    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
    // ANCHOR_END: plan_common
}

// Add
impl<VM: VMBinding> MyGC<VM> {
    // ANCHOR: plan_new
    fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        // Modify
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &MYGC_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&[]),
        };

        let res = MyGC {
            hi: AtomicBool::new(false),
            // ANCHOR: copyspace_new
            copyspace0: CopySpace::new(plan_args.get_space_args("copyspace0", true, VMRequest::discontiguous()), false),
            // ANCHOR_END: copyspace_new
            copyspace1: CopySpace::new(plan_args.get_space_args("copyspace1", true, VMRequest::discontiguous()), true),
            common: CommonPlan::new(plan_args),
        };

        res.verify_side_metadata_sanity();

        res
    }
    // ANCHOR_END: plan_new

    // ANCHOR: plan_space_access
    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn tospace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace1
        } else {
            &mut self.copyspace0
        }
    }

    pub fn fromspace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace0
        } else {
            &mut self.copyspace1
        }
    }
    // ANCHOR_END: plan_space_access
}
