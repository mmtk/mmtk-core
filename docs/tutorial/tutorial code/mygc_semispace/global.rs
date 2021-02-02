use super::gc_works::{MyGCCopyContext, MyGCProcessEdges}; // Add
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan; //Modify
use crate::plan::global::CommonPlan; // Add
use crate::plan::global::GcStatus; // Add
use crate::plan::mutator_context::Mutator;
use crate::plan::mygc::mutator::create_mygc_mutator;
use crate::plan::mygc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace; // Add
use crate::policy::space::Space;
use crate::scheduler::gc_works::*; // Add
use crate::scheduler::*; // Modify
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicBool, Ordering}; // Add 
use std::sync::Arc;
use enum_map::EnumMap;
// Remove #[allow(unused_imports)].
// Remove handle_user_collection_request.

pub type SelectedPlan<VM> = MyGC<VM>;

pub const ALLOC_MyGC: AllocationSemantics = AllocationSemantics::Default; // Add

// Modify
pub struct MyGC<VM: VMBinding> {
    pub hi: AtomicBool, 
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>, 
}

unsafe impl<VM: VMBinding> Sync for MyGC<VM> {}

impl<VM: VMBinding> Plan for MyGC<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = MyGCCopyContext<VM>; // Modify

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        // Modify
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        MyGC {
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    // Modify
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

    // Modify
    fn schedule_collection(&'static self, scheduler:&MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.unconstrained_works
            .add(StopMutators::<MyGCProcessEdges<VM>>::new());
        scheduler.prepare_stage.add(Prepare::new(self));
        scheduler.release_stage.add(Release::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        _mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_mygc_mutator(tls, self))
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    // Modify
    fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
        // Flips 'hi' to flip space definitions
        let hi = self.hi.load(Ordering::SeqCst); 
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    // Modify
    fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        self.fromspace().release();
    }

    // Modify
    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }
 
    // Modify
    fn get_pages_used(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    // Modify
    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    // Add
    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

// Add
impl<VM: VMBinding> MyGC<VM> {
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
}
