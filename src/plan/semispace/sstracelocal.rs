use ::plan::{TransitiveClosure, TraceLocal};
use ::plan::trace::Trace;
use ::util::{Address, ObjectReference};
use ::policy::space::Space;
use ::vm::VMScanning;
use ::vm::Scanning;
use std::sync::mpsc::Sender;
use crossbeam_deque::{Steal, Stealer};

use super::ss;
use ::plan::selected_plan::PLAN;

const PUSH_BACK_THRESHOLD: usize = 50;

pub struct SSTraceLocal {
    thread_id: usize,
    values: Vec<ObjectReference>,
    values_pool: (Stealer<ObjectReference>, Sender<ObjectReference>),
    root_locations: Vec<Address>,
    root_locations_pool: (Stealer<Address>, Sender<Address>),
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
        if self.values.len() >= PUSH_BACK_THRESHOLD {
            self.values_pool.1.send(object).unwrap();
        } else {
            self.values.push(object);
        }
    }
}

impl TraceLocal for SSTraceLocal {
    fn process_roots(&mut self) {
        loop {
            let slot = {
                if !self.root_locations.is_empty() {
                    self.root_locations.pop().unwrap()
                } else {
                    let work = self.root_locations_pool.0.steal();
                    match work {
                        Steal::Data(s) => s,
                        Steal::Empty => return,
                        Steal::Retry => continue
                    }
                }
            };
            self.process_root_edge(slot, true)
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

        // TODO Global empty or local empty
        // if !self.root_locations.is_empty() {
        self.process_roots();
        // }
        loop {
            let object = {
                if !self.values.is_empty() {
                    self.values.pop().unwrap()
                } else {
                    let work = self.values_pool.0.steal();
                    match work {
                        Steal::Data(o) => o,
                        Steal::Empty => return,
                        Steal::Retry => continue
                    }
                }
            };
            VMScanning::scan_object(self, object, id);
        }
    }

    fn release(&mut self) {}

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        if self.root_locations.len() >= PUSH_BACK_THRESHOLD {
            self.root_locations_pool.1.send(slot).unwrap();
        } else {
            self.root_locations.push(slot);
        }
    }
}

impl SSTraceLocal {
    pub fn new(ss_trace: &Trace) -> Self {
        SSTraceLocal {
            thread_id: 0,
            values: Vec::new(),
            values_pool: PLAN.ss_trace.get_value_pool(),
            root_locations: Vec::new(),
            root_locations_pool: PLAN.ss_trace.get_root_location_pool(),
        }
    }

    pub fn init(&mut self, thread_id: usize) {
        self.thread_id = thread_id;
    }
}