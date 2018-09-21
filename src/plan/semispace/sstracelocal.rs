use ::plan::{TransitiveClosure, TraceLocal};
use ::plan::trace::Trace;
use ::util::{Address, ObjectReference};
use ::policy::space::Space;
use ::vm::VMScanning;
use ::vm::Scanning;
use std::sync::mpsc::Sender;
use crossbeam_deque::{Steal, Stealer};

use super::ss;
use ::plan::semispace::PLAN;

use libc::c_void;

const PUSH_BACK_THRESHOLD: usize = 50;

pub struct SSTraceLocal {
    tls: *mut c_void,
    values: Vec<ObjectReference>,
    values_pool: (Stealer<ObjectReference>, Sender<ObjectReference>),
    root_locations: Vec<Address>,
    root_locations_pool: (Stealer<Address>, Sender<Address>),
}

impl TransitiveClosure for SSTraceLocal {
    fn process_edge(&mut self, slot: Address) {
        trace!("process_edge({:?})", slot);
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn process_node(&mut self, object: ObjectReference) {
        trace!("process_node({:?})", object);
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
        trace!("process_root_edge({:?}, {:?})", slot, untraced);
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        trace!("trace_object({:?})", object.to_address());
        let tls = self.tls;
        let plan_unsync = unsafe { &*PLAN.unsync.get() };

        if object.is_null() {
            trace!("trace_object: object is null");
            return object;
        }
        if plan_unsync.copyspace0.in_space(object) {
            trace!("trace_object: object in copyspace0");
            return plan_unsync.copyspace0.trace_object(self, object, ss::ALLOC_SS, tls);
        }
        if plan_unsync.copyspace1.in_space(object) {
            trace!("trace_object: object in copyspace1");
            return plan_unsync.copyspace1.trace_object(self, object, ss::ALLOC_SS, tls);
        }
        if plan_unsync.versatile_space.in_space(object) {
            trace!("trace_object: object in versatile_space");
            return plan_unsync.versatile_space.trace_object(self, object);
        }
        if plan_unsync.vm_space.in_space(object) {
            trace!("trace_object: object in boot space");
            return plan_unsync.vm_space.trace_object(self, object);
        }

        panic!("No special case for space in trace_object");
    }

    fn complete_trace(&mut self) {
        let id = self.tls;

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
        trace!("report_delayed_root_edge {:?} local len: {:?}", slot, self.root_locations.len());
        if self.root_locations.len() >= PUSH_BACK_THRESHOLD {
            trace!("self.root_locations_pool.1.send({:?})", slot);
            self.root_locations_pool.1.send(slot).unwrap();
            trace!("self.root_locations_pool.1.sent");
        } else {
            trace!("self.root_locations.push({:?})", slot);
            self.root_locations.push(slot);
        }
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        let unsync = unsafe { &(*PLAN.unsync.get()) };
        (unsync.hi && !unsync.copyspace0.in_space(obj)) ||
            (!unsync.hi && !unsync.copyspace1.in_space(obj))
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        }
        let unsync = unsafe { &(*PLAN.unsync.get()) };
        if unsync.copyspace0.in_space(object) {
            if unsync.hi {
                return unsync.copyspace0.is_live(object);
            } else {
                return true;
            }
        }
        if unsync.copyspace1.in_space(object) {
            if unsync.hi {
                return true;
            } else {
                return unsync.copyspace1.is_live(object);
            }
        }
        // FIXME is it actually alive?
        if unsync.versatile_space.in_space(object) {
            return true;
        }
        if unsync.vm_space.in_space(object) {
            return true;
        }
        panic!("Invalid space")
    }
}

impl SSTraceLocal {
    pub fn new(ss_trace: &Trace) -> Self {
        SSTraceLocal {
            tls: 0 as *mut c_void,
            values: Vec::new(),
            values_pool: PLAN.ss_trace.get_value_pool(),
            root_locations: Vec::new(),
            root_locations_pool: PLAN.ss_trace.get_root_location_pool(),
        }
    }

    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }
}
