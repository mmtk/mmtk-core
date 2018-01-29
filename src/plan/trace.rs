use ::util::{Address, ObjectReference};
use std::collections::VecDeque;

pub struct Trace {
    values: VecDeque<ObjectReference>,
    root_locations: VecDeque<Address>,
}

impl Trace {
    pub fn new() -> Self {
        Trace {
            values: VecDeque::new(),
            root_locations: VecDeque::new(),
        }
    }

    pub fn prepare(&mut self) {
        unimplemented!()
    }
}