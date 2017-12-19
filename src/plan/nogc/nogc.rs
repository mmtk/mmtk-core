use super::super::plan::default;

use ::policy::immortalspace::ImmortalSpace;
use ::plan::Plan;
use ::plan::controller_collector_context::ControllerCollectorContext;

use libc::c_void;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();
}

use super::NoGCMutator;
pub type SelectedMutator<'a> = NoGCMutator<'a>;
pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub control_collector_context: ControllerCollectorContext,
    space: ImmortalSpace,
}

impl Plan for NoGC {
    fn new() -> Self {
        NoGC {
            control_collector_context: ControllerCollectorContext::new(),
            space: ImmortalSpace::new(),
        }
    }

    fn gc_init(&self, heap_size: usize) {
        default::gc_init(&self.space, heap_size);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        default::bind_mutator::<NoGCMutator, ImmortalSpace>(thread_id, &self.space)
    }

    fn do_collection(&self) {
        panic!("GC triggered in NoGC plan");
    }
}