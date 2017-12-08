use ::util::{Address, ObjectReference};

pub trait TransitiveClosure {
    fn process_edge(&mut self, source: ObjectReference, slot: Address);
    fn process_node(&mut self, object: ObjectReference);
}