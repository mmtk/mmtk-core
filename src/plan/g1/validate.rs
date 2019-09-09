use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use util::{Address, ObjectReference};
use vm::*;
use plan::plan;
use plan::phase;
use plan::{TraceLocal, TransitiveClosure};
use plan::trace::Trace;
use util::queue::LocalQueue;
use vm::Scanning;
use vm::VMScanning;
use libc::c_void;
use std::marker::PhantomData;

pub const ENABLE: bool = false;

// const USE_LOG_BIT_FOR_MARKING: bool = true;

lazy_static! {
    static ref MARK_WORD_OFFSET: isize = VMObjectModel::GC_HEADER_OFFSET();
    static ref TRACE: Trace = Trace::new();
}

lazy_static! {
    pub static ref VALIDATION_PHASE: phase::Phase = phase::Phase::Complex(vec![
        // Prepare
        (phase::Schedule::Mutator,   phase::Phase::ValidatePrepare),
        (phase::Schedule::Global,    phase::Phase::ValidatePrepare),
        (phase::Schedule::Collector, phase::Phase::ValidatePrepare),
        // Roots
        (phase::Schedule::Complex,   plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global,    phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::Roots),
        (phase::Schedule::Collector, phase::Phase::Closure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Collector, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Collector, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs),
        // Release
        (phase::Schedule::Mutator,   phase::Phase::ValidateRelease),
        (phase::Schedule::Global,    phase::Phase::ValidateRelease),
        (phase::Schedule::Collector, phase::Phase::ValidateRelease),
    ], 0);
    pub static ref MARK_STATE: AtomicUsize = AtomicUsize::new(0);
    pub static ref IN_VALIDATION_PHASE: AtomicBool = AtomicBool::new(false);
}

pub fn schedule_validation_phase() -> (phase::Schedule, phase::Phase) {
    if ENABLE {
        (phase::Schedule::Complex, VALIDATION_PHASE.clone())
    } else {
        (phase::Schedule::Placeholder, phase::Phase::ValidatePlaceholder)
    }
}

#[inline]
pub fn prepare() {
    if ENABLE {
        MARK_STATE.fetch_add(1, Ordering::Relaxed);
        IN_VALIDATION_PHASE.store(true, Ordering::SeqCst);
    }
}

#[inline]
pub fn release() {
    if ENABLE {
        IN_VALIDATION_PHASE.store(false, Ordering::SeqCst);
    }
}

fn test_and_mark(object: ObjectReference) -> bool {
    // if USE_LOG_BIT_FOR_MARKING {
        // ::util::header_byte::attempt_unlog(object)
    // } else {
        let mark_word = object.to_address() + *MARK_WORD_OFFSET;
        let mark_state = MARK_STATE.load(Ordering::Relaxed);
        if unsafe { mark_word.load::<usize>() } != mark_state {
            unsafe { mark_word.store(mark_state) };
            true
        } else {
            false
        }
    // }
}

fn is_marked(object: ObjectReference) -> bool {
    // if USE_LOG_BIT_FOR_MARKING {
    //     ::util::header_byte::is_unlogged(object)
    // } else {
        let mark_word = object.to_address() + *MARK_WORD_OFFSET;
        let mark_state = MARK_STATE.load(Ordering::Relaxed);
        if unsafe { mark_word.load::<usize>() } != mark_state {
            unsafe { mark_word.store(mark_state) };
            true
        } else {
            false
        }
    // }
}
















pub trait Validator {
    fn validate_root(_slot: Address) {}
    fn validate_edge(_src: ObjectReference, _slot: Address, _obj: ObjectReference) {}
    fn validate_object(_obj: ObjectReference) {}
}

pub struct ValidateTraceLocal<V: Validator> {
    tls: *mut c_void,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    phantom: PhantomData<V>,
}

impl <V: Validator> TransitiveClosure for ValidateTraceLocal<V> {
    fn process_edge(&mut self, src: ObjectReference, slot: Address) {
        V::validate_object(src);
        let object: ObjectReference = unsafe { slot.load() };
        V::validate_edge(src, slot, object);
        self.trace_object(object);
    }

    fn process_node(&mut self, object: ObjectReference) {
        V::validate_object(object);
        self.values.enqueue(object);
    }
}

impl <V: Validator> TraceLocal for ValidateTraceLocal<V> {
    fn process_remembered_sets(&mut self) {}

    fn overwrite_reference_during_trace(&self) -> bool {
        false
    }

    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.dequeue() {
            self.process_root_edge(slot, true)
        }
        debug_assert!(self.root_locations.is_empty());
    }

    fn process_root_edge(&mut self, slot: Address, _untraced: bool) {
        V::validate_root(slot);
        let object: ObjectReference = unsafe { slot.load() };
        self.trace_object(object);
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if ENABLE {
            if object.is_null() {
                return object;
            }
            V::validate_object(object);
            if test_and_mark(object) {
                self.process_node(object);
            }
        }
        return object;
    }

    fn complete_trace(&mut self) {
        let id = self.tls;
        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        loop {
            while let Some(object) = self.values.dequeue() {
                V::validate_object(object);
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

    fn process_interior_edge(&mut self, _target: ObjectReference, _slot: Address, _root: bool) {
        unreachable!();
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        V::validate_root(slot);
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        V::validate_object(obj);
        true
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        use policy::space::Space;
        if object.is_null() {
            return false;
        } if super::PLAN.versatile_space.in_space(object) {
            true
        } else if super::PLAN.vm_space.in_space(object) {
            true
        } else {
            V::validate_object(object);
            is_marked(object)
        }
    }
}

impl <V: Validator> ValidateTraceLocal<V> {
    pub fn new() -> Self {
        Self {
            tls: 0 as *mut c_void,
            values: TRACE.values.spawn_local(),
            root_locations: TRACE.root_locations.spawn_local(),
            phantom: PhantomData,
        }
    }

    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }
}
