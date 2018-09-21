use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{Address, ObjectReference};
use ::vm::VMScanning;
use ::vm::Scanning;
use ::policy::space::Space;

use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashSet, LinkedList};
use ::plan::selected_plan::PLAN;
use ::plan::Plan;

use libc::c_void;

pub static INSIDE_SANITY: AtomicBool = AtomicBool::new(false);

pub struct SanityChecker {
    roots: Vec<Address>,
    values: LinkedList<ObjectReference>,
    refs: HashSet<ObjectReference>,
    tls: *mut c_void,
}

impl SanityChecker {
    pub fn new() -> Self {
        SanityChecker {
            roots: Vec::new(),
            values: LinkedList::new(),
            refs: HashSet::new(),
            tls: usize::max_value() as *mut c_void,
        }
    }

    pub fn check(&mut self, tls: *mut c_void) {
        self.tls = tls;
        INSIDE_SANITY.store(true, Ordering::Relaxed);
        println!("Sanity stackroots, collector");
        VMScanning::compute_thread_roots(self, tls);
        println!("Sanity stackroots, global");
        VMScanning::notify_initial_thread_scan_complete(false, tls);
        println!("Sanity roots, collector");
        VMScanning::compute_global_roots(self, tls);
        VMScanning::compute_static_roots(self, tls);
        VMScanning::compute_bootimage_roots(self, tls);
        println!("Sanity roots, global");
        VMScanning::reset_thread_counter();

        self.process_roots();
        self.complete_trace();

        self.roots.clear();
        self.values.clear();
        self.refs.clear();

        INSIDE_SANITY.store(false, Ordering::Relaxed);
        self.tls = usize::max_value() as *mut c_void;
    }
}

impl TransitiveClosure for SanityChecker{
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

impl TraceLocal for SanityChecker{
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
            if !PLAN.is_valid_ref(object) {
                panic!("Invalid reference")
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