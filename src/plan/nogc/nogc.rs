use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::controller_collector_context::ControllerCollectorContext;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;
use ::util::heap::VMRequest;

use std::cell::UnsafeCell;
use std::thread;
use libc::c_void;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();
}

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;
use util::conversions::bytes_to_pages;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub control_collector_context: ControllerCollectorContext,
    unsync: UnsafeCell<NoGCUnsync>,
}

unsafe impl Sync for NoGC {}

pub struct NoGCUnsync {
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
}