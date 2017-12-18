use ::plan::Plan;
use ::policy::copyspace::CopySpace;

use libc::c_void;

pub struct SemiSpace {
    hi: bool,
    copyspace0: CopySpace,
    copyspace1: CopySpace,
}

impl Plan for SemiSpace {
    fn new() -> Self {
        SemiSpace {
            hi: false,
            copyspace0: CopySpace {},
            copyspace1: CopySpace {},
        }
    }

    fn gc_init(&self, heap_size: usize) {
        unimplemented!();
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        unimplemented!();
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