use super::super::plan::default;

use std::thread::sleep;
use std::time;

use ::policy::space::Space;

use ::plan::semispace::ssmutator::SSMutator;
use ::plan::controller_collector_context::ControllerCollectorContext;

use ::plan::Plan;
use ::policy::copyspace::CopySpace;
use ::plan::Phase;
use libc::c_void;

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
            copyspace0: CopySpace::new(false),
            copyspace1: CopySpace::new(true),
        }
    }

    fn gc_init(&self, heap_size: usize) {
        self.copyspace0.init(heap_size/2);
        default::gc_init(&self.copyspace1, heap_size/2);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        default::bind_mutator(SSMutator::new(thread_id, self.fromspace()))
    }

    fn do_collection(&self) {
        println!("Collecting garbage, trust me...");
        sleep(time::Duration::from_millis(2000));
    }
}

impl SemiSpace {
    pub fn tospace(&self) -> &CopySpace {
        if self.hi {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace {
        if self.hi {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn collection_phase(&mut self, phase: Phase) {
        if let Phase::Prepare = phase {
            self.hi = !self.hi;
            self.copyspace0.prepare(self.hi);
            self.copyspace1.prepare(!self.hi);
        }
    }
}