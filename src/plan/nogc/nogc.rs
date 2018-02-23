use super::super::plan::default;

use ::policy::immortalspace::ImmortalSpace;
use ::plan::controller_collector_context::ControllerCollectorContext;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;

use libc::c_void;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();
}

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub control_collector_context: ControllerCollectorContext,
    space: UnsafeCell<ImmortalSpace>,
}

unsafe impl Sync for NoGC {}

impl Plan for NoGC {
    type MutatorT = NoGCMutator;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector;

    fn new() -> Self {
        NoGC {
            control_collector_context: ControllerCollectorContext::new(),
            space: UnsafeCell::new(ImmortalSpace::new()),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        default::gc_init(unsafe { &mut *(self.space.get()) });
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        default::bind_mutator(NoGCMutator::new(thread_id, unsafe { &*(self.space.get()) }))
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, thread_id: usize, phase: &Phase) {}
}