use crate::plan::{Phase, Plan};
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::space::Space;
use crate::util::heap::VMRequest;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;

use std::cell::UnsafeCell;

use super::NoGCCollector;
use super::NoGCMutator;
use super::NoGCTraceLocal;
use crate::plan::plan::BasePlan;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use std::sync::Arc;

pub type SelectedPlan<VM> = NoGC<VM>;

pub struct NoGC<VM: VMBinding> {
    pub unsync: UnsafeCell<NoGCUnsync<VM>>,
    pub base: BasePlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for NoGC<VM> {}

pub struct NoGCUnsync<VM: VMBinding> {
    pub nogc_space: ImmortalSpace<VM>,
}

impl<VM: VMBinding> Plan<VM> for NoGC<VM> {
    type MutatorT = NoGCMutator<VM>;
    type TraceLocalT = NoGCTraceLocal<VM>;
    type CollectorT = NoGCCollector<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        NoGC {
            unsync: UnsafeCell::new(NoGCUnsync {
                nogc_space: ImmortalSpace::new(
                    "nogc_space",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                ),
            }),
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

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<NoGCMutator<VM>> {
        Box::new(NoGCMutator::new(tls, self))
    }

    unsafe fn collection_phase(&self, _tls: OpaquePointer, _phase: &Phase) {
        unreachable!()
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.nogc_space.reserved_pages()
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.nogc_space.in_space(object) {
            true
        } else {
            self.base.is_valid_ref(object)
        }
    }

    fn is_bad_ref(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_in_space(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe { unsync.nogc_space.in_space(address.to_object_reference()) } {
            return true;
        }
        unsafe { self.base.in_base_space(address.to_object_reference()) }
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn get_immortal_space(&self) -> &'static ImmortalSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.nogc_space
    }
}
