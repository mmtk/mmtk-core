use plan::{TraceLocal, TransitiveClosure};
use plan::g1::PLAN;
use plan::trace::Trace;
use policy::space::Space;
use util::{Address, ObjectReference};
use util::queue::LocalQueue;
use vm::Scanning;
use vm::VMScanning;
use libc::c_void;
use super::g1;

#[derive(PartialEq)]
pub enum TraceKind {
  Mark, Evacuate
}

pub struct G1TraceLocal {
    tls: *mut c_void,
    kind: TraceKind,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
}

impl TransitiveClosure for G1TraceLocal {
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

impl TraceLocal for G1TraceLocal {
    fn overwrite_reference_during_trace(&self) -> bool {
        self.kind == TraceKind::Evacuate
    }

    fn process_roots(&mut self) {
        loop {
            match self.root_locations.dequeue() {
                Some(slot) => {
                    self.process_root_edge(slot, true)
                }
                None => {
                    break;
                }
            }
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

        if object.is_null() {
            object
        } else if PLAN.region_space.in_space(object) {
            match self.kind {
                TraceKind::Mark => PLAN.region_space.trace_mark_object(self, object),
                TraceKind::Evacuate => PLAN.region_space.trace_evacuate_object(self, object, g1::ALLOC_RS, tls),
            }
        } else if PLAN.versatile_space.in_space(object) {
            PLAN.versatile_space.trace_object(self, object)
        } else if PLAN.los.in_space(object) {
            PLAN.los.trace_object(self, object)
        } else if PLAN.vm_space.in_space(object) {
            PLAN.vm_space.trace_object(self, object)
        } else {
            unreachable!("{:?}", object)
        }
    }

    fn complete_trace(&mut self) {
        let id = self.tls;

        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        loop {
            match self.values.dequeue() {
                Some(object) => {
                    VMScanning::scan_object(self, object, id);
                }
                None => {
                    break;
                }
            }
        }
        debug_assert!(self.root_locations.is_empty());
        debug_assert!(self.values.is_empty());
    }

    fn release(&mut self) {
        // Reset the local buffer (throwing away any local entries).
        self.root_locations.reset();
        self.values.reset();
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
        trace!("report_delayed_root_edge {:?}", slot);
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        let unsync = unsafe { &(*PLAN.unsync.get()) };
        if g1::PLAN.region_space.in_space(obj) {
            self.kind == TraceKind::Mark
        } else {
            true
        }
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        } else if PLAN.region_space.in_space(object) {
            PLAN.region_space.is_live(object)
        } else if PLAN.versatile_space.in_space(object) {
            true
        } else if PLAN.los.in_space(object) {
            PLAN.los.is_live(object)
        } else if PLAN.vm_space.in_space(object) {
            true
        } else {
            unreachable!()
        }
    }
}

impl G1TraceLocal {
    pub fn new(kind: TraceKind, trace: &'static Trace) -> Self {
        G1TraceLocal {
            tls: 0 as *mut c_void,
            kind,
            values: trace.values.spawn_local(),
            root_locations: trace.root_locations.spawn_local(),
        }
    }

    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }
}
