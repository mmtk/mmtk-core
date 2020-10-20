use crate::plan::{Phase, Plan};
use crate::policy::space::Space;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::OpaquePointer;

use std::cell::UnsafeCell;

use super::NoGCCollector;
use super::NoGCTraceLocal;
use crate::plan::global::BasePlan;
use crate::plan::mutator_context::Mutator;
use crate::plan::nogc::mutator::create_nogc_mutator;
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::Allocator;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;

#[cfg(not(feature = "nogc_lock_free"))]
use crate::policy::immortalspace::ImmortalSpace as NoGCImmortalSpace;
#[cfg(feature = "nogc_lock_free")]
use crate::policy::lockfreeimmortalspace::LockFreeImmortalSpace as NoGCImmortalSpace;

pub type SelectedPlan<VM> = NoGC<VM>;

pub struct NoGC<VM: VMBinding> {
    pub unsync: UnsafeCell<NoGCUnsync<VM>>,
    pub base: BasePlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for NoGC<VM> {}

pub struct NoGCUnsync<VM: VMBinding> {
    pub nogc_space: NoGCImmortalSpace<VM>,
}

impl<VM: VMBinding> Plan<VM> for NoGC<VM> {
    type MutatorT = Mutator<VM, Self>;
    type TraceLocalT = NoGCTraceLocal<VM>;
    type CollectorT = NoGCCollector<VM>;

    fn new(
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
        );

        NoGC {
            unsync: UnsafeCell::new(NoGCUnsync { nogc_space }),
            base: BasePlan::new(vm_map, mmapper, options, heap),
        }
    }

    fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        self.base.gc_init(heap_size, vm_map);

        // FIXME correctly initialize spaces based on options
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.nogc_space.init(vm_map);
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<Mutator<VM, Self>> {
        Box::new(create_nogc_mutator(tls, self))
    }

    unsafe fn collection_phase(&self, _tls: OpaquePointer, _phase: &Phase) {
        unreachable!()
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<Allocator, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.nogc_space.reserved_pages()
    }

    fn handle_user_collection_request(&self, _tls: OpaquePointer, _force: bool) {
        println!("Warning: User attempted a collection request, but it is not supported in NoGC. The request is ignored.");
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn get_immortal_space(&self) -> &'static NoGCImmortalSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.nogc_space
    }
}
