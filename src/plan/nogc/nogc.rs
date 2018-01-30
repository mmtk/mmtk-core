use super::super::plan::default;

use ::policy::immortalspace::ImmortalSpace;
use ::plan::controller_collector_context::ControllerCollectorContext;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;

use libc::c_void;

lazy_static! {
    pub static ref PLAN: NoGC<'static> = NoGC::new();
}

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;

pub type SelectedPlan<'a> = NoGC<'a>;

pub struct NoGC<'a> {
    pub control_collector_context: ControllerCollectorContext<'a>,
    space: ImmortalSpace,
}

impl<'a> Plan for NoGC<'a> {
    type MutatorT = NoGCMutator<'a>;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector<'a>;

    fn new() -> Self {
        NoGC {
            control_collector_context: ControllerCollectorContext::new(),
            space: ImmortalSpace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        default::gc_init(&self.space, heap_size);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        default::bind_mutator(NoGCMutator::new(thread_id, &self.space))
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, phase: &Phase) {
        unimplemented!()
    }
}