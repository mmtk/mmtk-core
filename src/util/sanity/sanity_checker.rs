use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{Address, ObjectReference};
use ::vm::VMScanning;
use ::vm::Scanning;
use ::policy::space::Space;
use ::util::OpaquePointer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashSet, LinkedList};
use ::plan::Plan;
use ::plan::SelectedPlan;

use libc::c_void;

pub struct SanityChecker<'a> {
    roots: Vec<Address>,
    values: LinkedList<ObjectReference>,
    refs: HashSet<ObjectReference>,
    tls: OpaquePointer,
    plan: &'a SelectedPlan,
}

impl<'a> SanityChecker<'a> {
    pub fn new(tls: OpaquePointer, plan: &'a SelectedPlan) -> Self {
        SanityChecker {
            roots: Vec::new(),
            values: LinkedList::new(),
            refs: HashSet::new(),
            tls,
            plan,
        }
    }

    pub fn check(&mut self) {
        self.plan.common().enter_sanity();

        println!("Sanity stackroots, collector");
        VMScanning::compute_thread_roots(self, self.tls);
        println!("Sanity stackroots, global");
        VMScanning::notify_initial_thread_scan_complete(false, self.tls);
        println!("Sanity roots, collector");
        VMScanning::compute_global_roots(self, self.tls);
        VMScanning::compute_static_roots(self, self.tls);
        VMScanning::compute_bootimage_roots(self, self.tls);
        println!("Sanity roots, global");
        VMScanning::reset_thread_counter();

        self.process_roots();
        self.complete_trace();

        self.roots.clear();
        self.values.clear();
        self.refs.clear();

        self.plan.common().leave_sanity();
    }
}

impl<'a> TransitiveClosure for SanityChecker<'a> {
    fn process_edge(&mut self, slot: Address) {
        trace!("process_edge({:?})", slot);
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

impl<'a> TraceLocal for SanityChecker<'a> {
    fn process_roots(&mut self) {
        loop {
            if self.roots.is_empty() {
                break;
            }
            let slot = self.roots.pop().unwrap();
            self.process_root_edge(slot, true);
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
        if object.is_null() {
            return object;
        }

        if !self.refs.contains(&object) {
            if !self.plan.is_valid_ref(object) {
                panic!("Invalid reference {:?}", object);
            }
            // Object is not "marked"
            self.refs.insert(object); // "Mark" it
            self.process_node(object);
        }
        object
    }

    fn complete_trace(&mut self) {
        self.process_roots();

        loop {
            if self.values.is_empty() {
                break;
            }

            let object = self.values.pop_front().unwrap();
            let tls = self.tls;
            VMScanning::scan_object(self, object, tls);
        }
    }

    fn release(&mut self) {
        unimplemented!()
    }

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn overwrite_reference_during_trace(&self) -> bool {
        false
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        self.roots.push(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        return true;
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        unimplemented!()
    }
}