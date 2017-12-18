use std::thread;

use ::util::alloc::bumpallocator::BumpAllocator;
use ::util::alloc::allocator::Allocator;

use libc::c_void;

use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;

use ::plan::Plan;
use ::plan::controllercollectorcontext::ControllerCollectorContext;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();
}
pub type NoGCMutator<'a> = BumpAllocator<'a, ImmortalSpace>;
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
        self.space.init(heap_size);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        Box::into_raw(Box::new(NoGCMutator::new(thread_id, &self.space))) as *mut c_void
    }

    fn do_collection(&self) {
        // Copyright Yi Lin, 2017
        panic!("GC triggered while GC is disabled");
    }
}