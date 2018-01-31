use ::plan::{TransitiveClosure, TraceLocal};
use ::plan::trace::Trace;
use ::util::{Address, ObjectReference};
use std::collections::VecDeque;
use ::policy::space::Space;
use ::vm::VMScanning;
use ::vm::Scanning;
use crossbeam_deque::{Deque, Steal, Stealer};

use super::ss;
use ::plan::selected_plan::PLAN;

pub struct SSTraceLocal {
    thread_id: usize,
    root_locations: Stealer<Address>,
    values: Stealer<ObjectReference>,
}

impl TransitiveClosure for SSTraceLocal {
    fn process_edge(&mut self, slot: Address) {
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn process_node(&mut self, object: ObjectReference) {
        // FIXME
        // self.values.push_back(object);
        unimplemented!()
    }
}

impl TraceLocal for SSTraceLocal {
    fn process_roots(&mut self) {
        loop {
            match self.root_locations.steal() {
                Steal::Empty => break,
                Steal::Data(slot) => self.process_root_edge(slot, true),
                Steal::Retry => {}
            }
        }
    }

    fn process_root_edge(&mut self, slot: Address, untraced: bool) {
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        let plan_unsync = unsafe { &*PLAN.unsync.get() };

        if object.is_null() {
            return object;
        }
        if plan_unsync.copyspace0.in_space(object) {
            return plan_unsync.copyspace0.trace_object(self, object, ss::ALLOC_SS);
        }
        if plan_unsync.copyspace1.in_space(object) {
            return plan_unsync.copyspace1.trace_object(self, object, ss::ALLOC_SS);
        }
        if plan_unsync.versatile_space.in_space(object) {
            return plan_unsync.versatile_space.trace_object(self, object);
        }
        panic!("No special case for space in trace_object");
    }

    fn complete_trace(&mut self) {
        let id = self.thread_id;

        if !self.root_locations.is_empty() {
            self.process_roots();
        }

        loop {
            match self.values.steal() {
                Steal::Empty => break,
                Steal::Data(object) => VMScanning::scan_object(self, object, id),
                Steal::Retry => {}
            }
        }
    }

    fn release(&mut self) {
    }

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        // FIXME
        // self.root_locations.push_front(slot);
        unimplemented!()
    }
}

impl SSTraceLocal {
    pub fn new(ss_trace: &Trace) -> Self {
        SSTraceLocal {
            thread_id: 0,
            root_locations: ss_trace.root_locations.stealer(),
            values: ss_trace.values.stealer(),
        }
    }

    pub fn init(&mut self, thread_id: usize) {
        self.thread_id = thread_id;
    }
}