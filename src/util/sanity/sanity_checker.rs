use crate::plan::{TransitiveClosure, TraceLocal};
use crate::util::{Address, ObjectReference};
use crate::vm::Scanning;
use crate::util::OpaquePointer;

use std::collections::{HashSet, LinkedList};
use crate::plan::Plan;
use crate::plan::SelectedPlan;

use crate::vm::VMBinding;

pub struct SanityChecker<'a, VM: VMBinding> {
    roots: Vec<Address>,
    values: LinkedList<ObjectReference>,
    refs: HashSet<ObjectReference>,
    tls: OpaquePointer,
    plan: &'a SelectedPlan<VM>,
}

impl<'a, VM: VMBinding> SanityChecker<'a, VM> {
    pub fn new(tls: OpaquePointer, plan: &'a SelectedPlan<VM>) -> Self {
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
        VM::VMScanning::compute_thread_roots(self, self.tls);
        println!("Sanity stackroots, global");
        VM::VMScanning::notify_initial_thread_scan_complete(false, self.tls);
        println!("Sanity roots, collector");
        VM::VMScanning::compute_global_roots(self, self.tls);
        VM::VMScanning::compute_static_roots(self, self.tls);
        VM::VMScanning::compute_bootimage_roots(self, self.tls);
        println!("Sanity roots, global");
        VM::VMScanning::reset_thread_counter();

        self.process_roots();
        self.complete_trace();

        self.roots.clear();
        self.values.clear();
        self.refs.clear();

        self.plan.common().leave_sanity();
    }
}

impl<'a, VM: VMBinding> TransitiveClosure for SanityChecker<'a, VM> {
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

impl<'a, VM: VMBinding> TraceLocal for SanityChecker<'a, VM> {
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
            VM::VMScanning::scan_object(self, object, tls);
        }
    }

    fn release(&mut self) {
        unimplemented!()
    }

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, _root: bool) {
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

    fn will_not_move_in_current_collection(&self, _obj: ObjectReference) -> bool {
        true
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!()
    }
}