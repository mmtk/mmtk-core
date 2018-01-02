use ::plan::TransitiveClosure;
use ::util::{Address, ObjectReference};
use std::collections::VecDeque;

struct SSTraceLocal<'a> {
    root_locations: &'a mut VecDeque<Address>,
    values: &'a mut VecDeque<ObjectReference>,
}

impl<'a> TransitiveClosure for SSTraceLocal<'a> {
    fn process_edge(&mut self, slot: Address) {
        unimplemented!()
    }

    fn process_node(&mut self, object: ObjectReference) {
        self.values.push_back(object);
    }
}

impl<'a> SSTraceLocal<'a> {
    fn process_roots(&mut self) {
        unimplemented!()
    }
    fn process_root_edge(&mut self) {
        unimplemented!()
    }
}