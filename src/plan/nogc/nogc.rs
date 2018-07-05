use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::controller_collector_context::ControllerCollectorContext;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;
use ::util::heap::VMRequest;
use ::util::heap::layout::heap_layout::MMAPPER;
use ::util::Address;

use std::cell::UnsafeCell;
use std::thread;
use libc::c_void;

use std::mem::uninitialized;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();
}

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub control_collector_context: ControllerCollectorContext,
    unsync: UnsafeCell<NoGCUnsync>,
}

unsafe impl Sync for NoGC {}

pub struct NoGCUnsync {
    vm_space: ImmortalSpace,
    pub space: ImmortalSpace,
    pub total_pages: usize,
}

impl Plan for NoGC {
    type MutatorT = NoGCMutator;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector;

    fn new() -> Self {
        NoGC {
            control_collector_context: ControllerCollectorContext::new(),
            unsync: UnsafeCell::new(NoGCUnsync {
                vm_space: create_vm_space(),
                space: ImmortalSpace::new("nogc_space", true,
                                          VMRequest::RequestFraction {
                                              frac: 1.0,
                                              top: false,
                                          }),
                total_pages: 0,
            }
            ),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        // FIXME correctly initialize spaces based on options
        unsync.vm_space.init();
        unsync.space.init();

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0)
            });
        }
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(NoGCMutator::new(thread_id,
                                                &unsync.space))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, thread_id: usize, phase: &Phase) {}

    fn get_total_pages(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.total_pages
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.space.reserved_pages()
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            return true;
        }
        if unsync.vm_space.in_space(object) {
            return true;
        }
        return false;
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        false
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe {
            unsync.space.in_space(address.to_object_reference()) ||
            unsync.vm_space.in_space(address.to_object_reference())
        } {
            return MMAPPER.address_is_mapped(address);
        } else {
            return false;
        }
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            return unsync.space.is_movable();
        }
        if unsync.vm_space.in_space(object) {
            return unsync.vm_space.is_movable();
        }
        return true;
    }
}