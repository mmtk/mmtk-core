use ::plan::Plan;
use ::policy::copyspace::CopySpace;
use ::plan::phase::Phase;
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
            copyspace0: CopySpace::new(false),
            copyspace1: CopySpace::new(true),
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
    pub fn tospace(&mut self) -> &mut CopySpace {
        if self.hi {
            &mut self.copyspace1
        } else {
            &mut self.copyspace0
        }
    }

    pub fn fromspace(&mut self) -> &mut CopySpace {
        if self.hi {
            &mut self.copyspace0
        } else {
            &mut self.copyspace1
        }
    }

    pub fn collection_phase(&mut self, phase: Phase) {
        match phase {
            Phase::Prepare => {
                self.hi = !self.hi;
                self.copyspace0.prepare(self.hi);
                self.copyspace1.prepare(!self.hi);
            }
            _ => { unimplemented!() }
        }
    }
}