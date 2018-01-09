use ::util::{Address, ObjectReference};

pub trait TransitiveClosure {
    // The signature of this function changes during the port
    // because the argument `ObjectReference source` is never used in the original version
    // See issue #5
    fn process_edge(&mut self, slot: Address);
    fn process_node(&mut self, object: ObjectReference);
}

pub struct VirtualTransitiveClosure {}

impl TransitiveClosure for VirtualTransitiveClosure {
    fn process_edge(&mut self, slot: Address) {
        trace!("process_edge(slot: {:#?})", slot);
    }

    fn process_node(&mut self, object: ObjectReference) {
        trace!("process_node(object: {:#?})", object);
    }
}