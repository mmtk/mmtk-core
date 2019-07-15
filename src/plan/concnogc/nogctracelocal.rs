use ::plan::{TraceLocal, TransitiveClosure};
use super::PLAN;
use ::plan::trace::Trace;
use ::util::{Address, ObjectReference};
use ::util::queue::LocalQueue;
use vm::*;
use libc::c_void;

pub struct NoGCTraceLocal {
    tls: *mut c_void,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    pub modbuf: LocalQueue<'static, ObjectReference>,
}

impl TransitiveClosure for NoGCTraceLocal {
    fn process_edge(&mut self, slot: Address) {
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn process_node(&mut self, object: ObjectReference) {
        self.values.enqueue(object);
    }
}

impl TraceLocal for NoGCTraceLocal {
    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.dequeue() {
            self.process_root_edge(slot, true)
        }
        debug_assert!(self.root_locations.is_empty());
    }

    fn process_root_edge(&mut self, slot: Address, _untraced: bool) {
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

        let mark_slot = object.to_address() + (VMObjectModel::GC_HEADER_OFFSET() + 2isize);
        if unsafe { mark_slot.load::<u16>() } != PLAN.mark_state {
            unsafe { mark_slot.store(PLAN.mark_state as u16) };
            self.process_node(object)
        }
        return object
    }

    fn complete_trace(&mut self) {
        let id = self.tls;
        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        loop {
            while let Some(object) = self.values.dequeue() {
                VMScanning::scan_object(self, object, id);
            }
            self.process_remembered_sets();
            if self.values.is_empty() {
                break;
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

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, _root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, _obj: ObjectReference) -> bool {
        return true;
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        return !object.is_null();
    }

    fn process_remembered_sets(&mut self) {
        while let Some(obj) = self.modbuf.dequeue() {
            self.trace_object(obj);
        }
    }
}

impl NoGCTraceLocal {
    pub fn new(trace: &'static Trace) -> Self {
        NoGCTraceLocal {
            tls: 0 as *mut c_void,
            values: trace.values.spawn_local(),
            root_locations: trace.root_locations.spawn_local(),
            modbuf: PLAN.modbuf_pool.spawn_local()
        }
    }

    pub fn flush(&mut self) {
        self.values.flush();
        self.root_locations.flush();
    }


    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }

    pub fn incremental_trace(&mut self, work_limit: usize) -> bool {
        let mut units = 0;
        while let Some(v) = self.values.dequeue() {
            VMScanning::scan_object(self, v, self.tls);
            units += 1;
            if units >= work_limit {
                break;
            }
        }
        return self.values.is_empty();
    }

}
