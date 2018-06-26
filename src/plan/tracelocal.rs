use ::plan::TransitiveClosure;
use ::util::{ObjectReference, Address};

pub trait TraceLocal: TransitiveClosure {
    fn process_roots(&mut self);
    fn process_root_edge(&mut self, slot: Address, untraced: bool);
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;
    fn complete_trace(&mut self);
    fn release(&mut self);
    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool);
    fn overwrite_reference_during_trace(&self) -> bool {
        true
    }

    fn report_delayed_root_edge(&mut self, slot: Address);
    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool;
    fn get_forwarded_reference(&mut self, object: ObjectReference) -> ObjectReference {
        self.trace_object(object)
    }
}