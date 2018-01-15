use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{Address, ObjectReference};
use std::collections::VecDeque;
use ::policy::space::Space;
use ::vm::VMScanning;
use ::vm::Scanning;

use super::ss;
use ::plan::selected_plan::PLAN;

pub struct SSTraceLocal {
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
        if object.is_null() {
            return object;
        }
        if PLAN.copyspace0.in_space(object) {
            return PLAN.copyspace0.trace_object(self, object, ss::ALLOC_SS);
        }
        if PLAN.copyspace1.in_space(object) {
            return PLAN.copyspace1.trace_object(self, object, ss::ALLOC_SS);
        }
        if PLAN.versatile_space.in_space(object){
            return PLAN.versatile_space.trace_object(self, object);
        }
        panic!("No special case for space in trace_object");
    }

    fn complete_trace(&mut self) {
        if !self.root_locations.is_empty() {
            self.process_roots();
        }
        while let Some(object) = self.values.pop_front() {
            VMScanning::scan_object(self, object);
        }
        while !self.values.is_empty() {
            while let Some(object) = self.values.pop_front() {
                VMScanning::scan_object(self, object);
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
}

impl SSTraceLocal {
    pub fn new() -> Self {
        SSTraceLocal {
            root_locations: VecDeque::new(),
            values: VecDeque::new(),
        }
    }
}