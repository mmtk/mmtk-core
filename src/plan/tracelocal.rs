use ::plan::TransitiveClosure;
use ::util::{ObjectReference, Address};

pub trait TraceLocal: TransitiveClosure {
    fn process_roots(&mut self);
    fn process_root_edge(&mut self, slot: Address, untraced: bool);
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;
    fn complete_trace(&mut self);
    fn release(&mut self);
}