use ::plan::{TraceLocal, TransitiveClosure};
use super::PLAN;
use ::plan::trace::Trace;
use ::policy::space::Space;
use ::util::{Address, ObjectReference};
use ::util::queue::LocalQueue;
use ::vm::Scanning;
use ::vm::VMScanning;
use libc::c_void;
use super::ss;
use util::heap::layout::heap_layout::VM_MAP;
use std::sync::atomic::{AtomicUsize, Ordering};
use plan::plan::Plan;

fn validate(o: ObjectReference) {
    assert!(!PLAN.fromspace().in_space(o), "Object in from space");
    assert!(PLAN.is_valid_ref(o), "Object is invalid ref");
    assert!(PLAN.is_mapped_object(o), "Object is not mapped {:?}", o);
    // assert!(!::util::forwarding_word::is_forwarded_or_being_forwarded(o), "Object is forwarded {:?}", o);
}

#[inline(always)]
fn validate_slot(slot: Address) {
    if super::validate::active() {
        let o: ObjectReference = unsafe { slot.load() };
        if !o.is_null() {
            assert!(PLAN.is_valid_ref(o), "Slot {:?} in space#{} points to an invalid object", slot, VM_MAP.get_descriptor_for_address(slot));
        }
    }
}

pub struct SSTraceLocal {
    tls: *mut c_void,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    pub modbuf: LocalQueue<'static, ObjectReference>,
}

impl TransitiveClosure for SSTraceLocal {
    fn process_edge(&mut self, src: ObjectReference, slot: Address) {
        validate_slot(slot);
        let a = unsafe { ::std::mem::transmute::<Address, &AtomicUsize>(slot) };
        loop {
            let object: ObjectReference = unsafe { slot.load() };
            let new_object = self.trace_object(object);
            if object == new_object {
                return
            }
            if a.compare_and_swap(object.to_address().as_usize(), new_object.to_address().as_usize(), Ordering::Relaxed) == object.to_address().as_usize() {
                return
            }
        }
    }

    fn process_node(&mut self, object: ObjectReference) {
        self.values.enqueue(object);
    }
}

impl TraceLocal for SSTraceLocal {
    fn process_remembered_sets(&mut self) {
        while let Some(obj) = self.modbuf.dequeue() {
            self.trace_object(obj);
        }
    }

    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.dequeue() {
            self.process_root_edge(slot, true)
        }
        debug_assert!(self.root_locations.is_empty());
    }

    fn process_root_edge(&mut self, slot: Address, _untraced: bool) {
        validate_slot(slot);
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
        // validate(object);
        if super::validate::active() {
            super::validate::trace_validate_object(self, object, validate)
        } else {
            if PLAN.copyspace0.in_space(object) {
                PLAN.copyspace0.trace_object(self, object, ss::ALLOC_SS, self.tls)
            } else if PLAN.copyspace1.in_space(object) {
                PLAN.copyspace1.trace_object(self, object, ss::ALLOC_SS, self.tls)
            } else if PLAN.versatile_space.in_space(object) {
                PLAN.versatile_space.trace_object(self, object)
            } else if PLAN.vm_space.in_space(object) {
                PLAN.vm_space.trace_object(self, object)
            } else if PLAN.los.in_space(object) {
                PLAN.los.trace_object(self, object)
            } else {
                panic!("No special case for space in trace_object")
            }
        }
    }

    fn complete_trace(&mut self) {
        let id = self.tls;
        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        loop {
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
            debug_assert!(false);
            debug_assert!(!(new_target.to_address() + offset).is_zero());
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        validate_slot(slot);
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        if obj.is_null() {
            return false;
        }
        (PLAN.hi && !PLAN.copyspace0.in_space(obj)) || (!PLAN.hi && !PLAN.copyspace1.in_space(obj))
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        }
        if PLAN.copyspace0.in_space(object) {
            if PLAN.hi {
                PLAN.copyspace0.is_live(object)
            } else {
                true
            }
        } else if PLAN.copyspace1.in_space(object) {
            if PLAN.hi {
                true
            } else {
                PLAN.copyspace1.is_live(object)
            }
        } else if PLAN.versatile_space.in_space(object) {
            true
        } else if PLAN.vm_space.in_space(object) {
            true
        } else if PLAN.los.in_space(object) {
            true
        } else {
            panic!("Invalid space")
        }
    }
}

impl SSTraceLocal {
    pub fn new(ss_trace: &'static Trace) -> Self {
        SSTraceLocal {
            tls: 0 as *mut c_void,
            values: ss_trace.values.spawn_local(),
            root_locations: ss_trace.root_locations.spawn_local(),
            modbuf: PLAN.modbuf_pool.spawn_local()
        }
    }

    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }

    pub fn flush(&mut self) {
        self.values.flush();
        self.root_locations.flush();
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
