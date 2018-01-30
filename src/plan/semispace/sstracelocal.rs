use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{Address, ObjectReference};
use std::collections::VecDeque;
use ::policy::space::Space;
use ::vm::VMScanning;
use ::vm::Scanning;

use super::ss;
use ::plan::selected_plan::PLAN;

pub struct SSTraceLocal {
    thread_id: usize,
    root_locations: VecDeque<Address>,
    values: VecDeque<ObjectReference>,
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
        self.values.push_back(object);
    }
}

impl TraceLocal for SSTraceLocal {
    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.pop_front() {
            self.process_root_edge(slot, true);
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
        if plan_unsync.versatile_space.in_space(object){
            return plan_unsync.versatile_space.trace_object(self, object);
        }
        panic!("No special case for space in trace_object");
    }

    fn complete_trace(&mut self) {
        let id = self.thread_id;

        if !self.root_locations.is_empty() {
            self.process_roots();
        }
        while let Some(object) = self.values.pop_front() {
            VMScanning::scan_object(self, object, id);
        }
        while !self.values.is_empty() {
            while let Some(object) = self.values.pop_front() {
                VMScanning::scan_object(self, object, id);
            }
        }
    }

    fn release(&mut self) {
        self.values.clear();
        self.root_locations.clear();
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
        self.root_locations.push_front(slot);
    }
}

impl SSTraceLocal {
    pub fn new() -> Self {
        SSTraceLocal {
            thread_id: 0,
            root_locations: VecDeque::new(),
            values: VecDeque::new(),
        }
    }

    pub fn init(&mut self, thread_id: usize) {
        self.thread_id = thread_id;
    }
}