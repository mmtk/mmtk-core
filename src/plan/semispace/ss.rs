use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::bumpallocator::BumpAllocator;

use ::plan::controllercollectorcontext::ControllerCollectorContext;

use ::plan::Plan;
use ::policy::copyspace::CopySpace;

use libc::c_void;

pub type SSMutator<'a> = BumpAllocator<'a,ImmortalSpace>;
pub type SelectedMutator<'a> = SSMutator<'a>;
pub type SelectedPlan = SemiSpace;

lazy_static! {
    pub static ref PLAN: SemiSpace = SemiSpace::new();
}

pub struct SemiSpace {
    pub control_collector_context: ControllerCollectorContext,
    hi: bool,
    copyspace0: CopySpace,
    copyspace1: CopySpace,
}

impl Plan for SemiSpace {
    fn new() -> Self {
        SemiSpace {
            control_collector_context: ControllerCollectorContext::new(),
            hi: false,
            copyspace0: CopySpace {},
            copyspace1: CopySpace {},
        }
    }

    fn gc_init(&self, heap_size: usize) {
        panic!("Not implemented");
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        panic!("Not implemented");
    }

    fn do_collection(&self) {
        println!("Collecting garbage, trust me...");
    }
}

impl SemiSpace {
    fn tospace(&mut self) -> &mut CopySpace {
        if self.hi {
            &mut self.copyspace1
        } else {
            &mut self.copyspace0
        }
    }
    fn fromspace(&mut self) -> &mut CopySpace {
        if self.hi {
            &mut self.copyspace0
        } else {
            &mut self.copyspace1
        }
    }
}