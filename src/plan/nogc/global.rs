use crate::mmtk::MMTK;
use crate::plan::global::{BasePlan, NoCopy};
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::GCWorkerLocal;
use crate::scheduler::GCWorkerLocalPtr;
use crate::scheduler::MMTkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;

#[cfg(not(feature = "nogc_lock_free"))]
use crate::policy::immortalspace::ImmortalSpace as NoGCImmortalSpace;
#[cfg(feature = "nogc_lock_free")]
use crate::policy::lockfreeimmortalspace::LockFreeImmortalSpace as NoGCImmortalSpace;

pub struct NoGC<VM: VMBinding> {
    pub base: BasePlan<VM>,
    pub nogc_space: NoGCImmortalSpace<VM>,
}

pub const NOGC_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

impl<VM: VMBinding> Plan for NoGC<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &NOGC_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = NoCopy::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.base.gc_init(heap_size, vm_map, scheduler);

        // FIXME correctly initialize spaces based on options
        self.nogc_space.init(&vm_map);
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn prepare(&self, _tls: OpaquePointer) {
        unreachable!()
    }

    fn release(&self, _tls: OpaquePointer, _mmtk: &'static MMTK<VM>) {
        unreachable!()
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, _scheduler: &MMTkScheduler<VM>) {
        unreachable!("GC triggered in nogc")
    }

    fn get_pages_used(&self) -> usize {
        self.nogc_space.reserved_pages()
    }

    fn handle_user_collection_request(&self, _tls: OpaquePointer, _force: bool) {
        println!("Warning: User attempted a collection request, but it is not supported in NoGC. The request is ignored.");
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        #[cfg(not(feature = "nogc_lock_free"))]
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        #[cfg(feature = "nogc_lock_free")]
        let heap = HeapMeta::new(HEAP_START, HEAP_END);

        #[cfg(feature = "nogc_lock_free")]
        let nogc_space =
            NoGCImmortalSpace::new("nogc_space", cfg!(not(feature = "nogc_no_zeroing")));
        #[cfg(not(feature = "nogc_lock_free"))]
        let nogc_space = NoGCImmortalSpace::new(
            "nogc_space",
            true,
            VMRequest::discontiguous(),
            vm_map,
            mmapper,
            &mut heap,
            &NOGC_CONSTRAINTS,
        );

        NoGC {
            nogc_space,
            base: BasePlan::new(vm_map, mmapper, options, heap, &NOGC_CONSTRAINTS),
        }
    }
}
