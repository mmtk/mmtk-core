//! Unsynchronized thread-local trace mechanism (superseded by [ProcessEdgesWork](crate::scheduler::gc_work::ProcessEdgesWork)).

use crate::plan::ObjectQueue;
use crate::util::{Address, ObjectReference};

/// This trait and its global counterpart implement the core
/// functionality for a transitive closure over the heap graph. This trait
/// specifically implements the unsynchronized thread-local component
/// (ie the 'fast-path') of the trace mechanism.
pub trait TraceLocal: ObjectQueue {
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
    fn get_forwarded_referent(&mut self, object: ObjectReference) -> ObjectReference {
        self.get_forwarded_reference(object)
    }
    fn retain_referent(&mut self, object: ObjectReference) -> ObjectReference {
        self.trace_object(object)
    }
}
