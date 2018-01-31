use ::util::{Address, ObjectReference};
use crossbeam_deque::{Deque, Steal};

pub struct Trace {
    pub values: Deque<ObjectReference>,
    pub root_locations: Deque<Address>,
}

impl Trace {
    pub fn new() -> Self {
        Trace {
            values: Deque::new(),
            root_locations: Deque::new(),
        }
    }

    pub fn prepare(&mut self) {
        // FIXME
    }
}