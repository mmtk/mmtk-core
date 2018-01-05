use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{Address, ObjectReference};
use std::collections::VecDeque;
use ::policy::space::Space;

use super::ss;
use ::plan::selected_plan::PLAN;

pub struct SSTraceLocal {
    root_locations: VecDeque<Address>,
    values: VecDeque<ObjectReference>,
}

impl TransitiveClosure for SSTraceLocal {
    fn process_edge(&mut self, slot: Address) {
        unimplemented!()
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
        let object: ObjectReference = if untraced {
            unsafe { slot.load() }
        } else {
            unimplemented!()
        };
        let new_object = self.trace_object(object);
        // FIXME: overwriteReferenceDuringTrace
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
        unimplemented!()
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