use super::ss;
use crate::plan::semispace::SemiSpace;
use crate::plan::{TraceLocal, TransitiveClosure};
use crate::policy::space::Space;
use crate::policy::space::SFT;
use crate::util::queue::LocalQueue;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::Scanning;
use crate::vm::VMBinding;

pub struct SSTraceLocal<VM: VMBinding> {
    tls: OpaquePointer,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    plan: &'static SemiSpace<VM>,
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
            return plan_unsync
                .copyspace0
                .trace_object(self, object, ss::ALLOC_SS, tls);
        }
        if plan_unsync.copyspace1.in_space(object) {
            trace!("trace_object: object in copyspace1");
            return plan_unsync
                .copyspace1
                .trace_object(self, object, ss::ALLOC_SS, tls);
        }
        if self.plan.common.in_common_space(object) {
            return self.plan.common.trace_object(self, object);
        }
        self.plan.common.trace_object(self, object)
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
        (unsync.hi && !unsync.copyspace0.in_space(obj))
            || (!unsync.hi && !unsync.copyspace1.in_space(obj))
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
        if self.plan.common.in_common_space(object) {
            return self.plan.common.is_live(object);
        }
        self.plan.common.is_live(object)
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
