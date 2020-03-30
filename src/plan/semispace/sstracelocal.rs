use ::plan::{TraceLocal, TransitiveClosure};
use ::policy::space::Space;
use ::util::{Address, ObjectReference};
use ::util::queue::LocalQueue;
use ::vm::Scanning;
use super::ss;
use util::OpaquePointer;
use plan::semispace::SemiSpace;
use vm::VMBinding;

pub struct SSTraceLocal<VM: VMBinding> {
    tls: OpaquePointer,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    plan: &'static SemiSpace<VM>
}

impl<VM: VMBinding> TransitiveClosure for SSTraceLocal<VM> {
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
        self.values.enqueue(object);
    }
}

impl<VM: VMBinding> TraceLocal for SSTraceLocal<VM> {
    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.dequeue() {
            self.process_root_edge(slot, true);
        }
        debug_assert!(self.root_locations.is_empty());
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
        let plan_unsync = unsafe { &*self.plan.unsync.get() };

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
        if plan_unsync.vm_space.is_some() && plan_unsync.vm_space.as_ref().unwrap().in_space(object) {
            trace!("trace_object: object in boot space");
            return plan_unsync.vm_space.as_ref().unwrap().trace_object(self, object);
        }
        if plan_unsync.los.in_space(object) {
            trace!("trace_object: object in los");
            return plan_unsync.los.trace_object(self, object);
        }

        panic!("No special case for space in trace_object");
    }

    fn complete_trace(&mut self) {
        let id = self.tls;

        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        while let Some(object) = self.values.dequeue() {
            VM::VMScanning::scan_object(self, object, id);
        }
        debug_assert!(self.root_locations.is_empty());
        debug_assert!(self.values.is_empty());
    }

    fn release(&mut self) {
        // Reset the local buffer (throwing away any local entries).
        self.root_locations.reset();
        self.values.reset();
    }

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, _root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        trace!("report_delayed_root_edge {:?}", slot);
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        let unsync = unsafe { &(*self.plan.unsync.get()) };
        (unsync.hi && !unsync.copyspace0.in_space(obj)) ||
            (!unsync.hi && !unsync.copyspace1.in_space(obj))
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        }
        let unsync = unsafe { &(*self.plan.unsync.get()) };
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
        if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(object) {
            return true;
        }
        if unsync.los.in_space(object) {
            return true;
        }
        panic!("Invalid space")
    }
}

impl<VM: VMBinding> SSTraceLocal<VM> {
    pub fn new(ss: &'static SemiSpace<VM>) -> Self {
        let ss_trace = ss.get_sstrace();
        SSTraceLocal {
            tls: OpaquePointer::UNINITIALIZED,
            values: ss_trace.values.spawn_local(),
            root_locations: ss_trace.root_locations.spawn_local(),
            plan: ss,
        }
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }
}
