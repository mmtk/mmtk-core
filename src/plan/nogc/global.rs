use crate::plan::Plan;
use crate::policy::space::Space;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::OpaquePointer;
use crate::plan::global::{BasePlan, NoCopy};
use crate::plan::mutator_context::Mutator;
use crate::plan::nogc::mutator::create_nogc_mutator;
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::Allocator;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;
use crate::mmtk::MMTK;
use crate::scheduler::MMTkScheduler;

#[cfg(not(feature = "nogc_lock_free"))]
use crate::policy::immortalspace::ImmortalSpace as NoGCImmortalSpace;
#[cfg(feature = "nogc_lock_free")]
use crate::policy::lockfreeimmortalspace::LockFreeImmortalSpace as NoGCImmortalSpace;

pub type SelectedPlan<VM> = NoGC<VM>;

pub struct NoGC<VM: VMBinding> {
    pub base: BasePlan<VM>,
    pub nogc_space: NoGCImmortalSpace<VM>,
}

unsafe impl<VM: VMBinding> Sync for NoGC<VM> {}

impl<VM: VMBinding> Plan for NoGC<VM> {
    type VM = VM;
    type Mutator = Mutator<VM, Self>;
    type CopyContext = NoCopy<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<Self::VM>,
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
        );

        NoGC {
            nogc_space,
            base: BasePlan::new(vm_map, mmapper, options, heap),
        }
    }

    fn gc_init(&mut self, heap_size: usize, mmtk: &'static MMTK<VM>) {
        self.base.gc_init(heap_size, mmtk);

        // FIXME correctly initialize spaces based on options
        self.nogc_space.init(&mmtk.vm_map);
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<Mutator<VM, Self>> {
        Box::new(create_nogc_mutator(tls, self))
    }

    fn prepare(&self, _tls: OpaquePointer) {
        unreachable!()
    }

    fn release(&self, _tls: OpaquePointer) {
        unreachable!()
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<Allocator, AllocatorSelector> {
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
