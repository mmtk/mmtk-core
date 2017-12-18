use ::plan::TransitiveClosure;
use ::util::{Address, ObjectReference};

struct SSTraceLocal {}

impl TransitiveClosure for SSTraceLocal {
    fn process_edge(&mut self, source: ObjectReference, slot: Address) {
        unimplemented!()
    }

    fn process_node(&mut self, object: ObjectReference) {
        unimplemented!()
    }
}