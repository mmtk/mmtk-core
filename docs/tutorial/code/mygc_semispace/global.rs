// ANCHOR: imports
use super::gc_work::{MyGCCopyContext, MyGCProcessEdges}; // Add
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan; //Modify
use crate::plan::global::CommonPlan; // Add
use crate::plan::global::GcStatus; // Add
use crate::plan::mygc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace; // Add
use crate::policy::space::Space;
use crate::scheduler::gc_work::*; // Add
use crate::scheduler::*; // Modify
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::{MetadataSanity, MetadataContext};
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering}; // Add
use std::sync::Arc;
// Remove #[allow(unused_imports)].
// Remove handle_user_collection_request.
// ANCHOR_END: imports

pub type SelectedPlan<VM> = MyGC<VM>;

pub const ALLOC_MyGC: AllocationSemantics = AllocationSemantics::Default; // Add

// Modify
// ANCHOR: plan_def
pub struct MyGC<VM: VMBinding> {
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
}
// ANCHOR_END: plan_def

// ANCHOR: constraints
pub const MYGC_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    ..PlanConstraints::default()
};
// ANCHOR_END: constraints

impl<VM: VMBinding> Plan for MyGC<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &MYGC_CONSTRAINTS
    }

    // ANCHOR: create_worker_local
    fn create_worker_local(
        &self,
        tls: VMWorkerThread,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = MyGCCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }
    // ANCHOR_END: create_worker_local

    // Modify
    // ANCHOR: gc_init
    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.copyspace0.init(&vm_map);
        self.copyspace1.init(&vm_map);
    }
    // ANCHOR_END: gc_init

    // Modify
    // ANCHOR: schedule_collection
    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MyGCProcessEdges<VM>>::new());
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }
    // ANCHOR_END: schedule_collection

    // ANCHOR: collection_required()
    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
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
    }
    // ANCHOR_END: prepare

    // Modify
    // ANCHOR: release
    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.fromspace().release();
    }
    // ANCHOR_END: release

    // Modify
    // ANCHOR: plan_get_collection_reserve
    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }
    // ANCHOR_END: plan_get_collection_reserve

    // Modify
    // ANCHOR: plan_get_pages_used
    fn get_pages_used(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_pages_used()
    }
    // ANCHOR_END: plan_get_pages_used

    // Modify
    // ANCHOR: plan_base
    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
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
    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<VM>,
    ) -> Self {
        // Modify
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let global_metadata_specs = MetadataContext::new_global_specs(&[]);

        let res = MyGC {
            hi: AtomicBool::new(false),
            // ANCHOR: copyspace_new
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            // ANCHOR_END: copyspace_new
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap, &MYGC_CONSTRAINTS, global_metadata_specs.clone()),
        };

        let mut metadata_sanity_checker = MetadataSanity::new();
        res.common.verify_metadata_sanity(&mut metadata_sanity_checker);
        res.copyspace0.verify_metadata_sanity(&mut metadata_sanity_checker);
        res.copyspace1.verify_metadata_sanity(&mut metadata_sanity_checker);

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
    // ANCHOR_END: plan_space_access
}
