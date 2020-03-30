use ::plan::transitive_closure::TransitiveClosure;
use ::util::address::{Address, ObjectReference};
use ::plan::tracelocal::TraceLocal;
use vm::VMBinding;
use std::marker::PhantomData;

pub struct NoGCTraceLocal<VM: VMBinding> {
    p: PhantomData<VM>
}

impl<VM: VMBinding> TransitiveClosure for NoGCTraceLocal<VM> {
    fn process_edge(&mut self, _slot: Address) {
        unreachable!();
    }

    fn process_node(&mut self, _object: ObjectReference) {
        unreachable!()
    }
}

impl<VM: VMBinding> TraceLocal for NoGCTraceLocal<VM> {
    fn process_roots(&mut self) {
        unreachable!();
    }

    fn process_root_edge(&mut self, _slot: Address, _untraced: bool) {
        unreachable!();
    }

    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        unreachable!();
    }

    fn complete_trace(&mut self) {
        unreachable!();
    }

    fn release(&mut self) {
        unreachable!();
    }

    fn process_interior_edge(&mut self, _target: ObjectReference, _slot: Address, _root: bool) {
        unreachable!()
    }
    fn report_delayed_root_edge(&mut self, _slot: Address) {
        unreachable!()
    }

    fn will_not_move_in_current_collection(&self, _obj: ObjectReference) -> bool {
        true
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
}

impl<VM: VMBinding> NoGCTraceLocal<VM> {
    pub fn new() -> Self {
        Self {
            p: PhantomData
        }
    }
}

impl<VM: VMBinding> Default for NoGCTraceLocal<VM> {
    fn default() -> Self {
        Self::new()
    }
}