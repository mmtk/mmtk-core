use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use util::ObjectReference;
use util::constants::*;
use util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END};
use vm::*;
use plan::plan;
use plan::TransitiveClosure;
use plan::phase;

pub const ENABLE: bool = false;

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
        (phase::Schedule::Collector, phase::Phase::ValidateClosure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Collector, phase::Phase::ValidateClosure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Collector, phase::Phase::ValidateClosure),
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

#[inline(always)]
pub fn active() -> bool {
    if ENABLE {
        IN_VALIDATION_PHASE.load(Ordering::Relaxed)
    } else {
        false
    }
}

#[inline]
pub fn release() {
    if ENABLE {
        IN_VALIDATION_PHASE.store(false, Ordering::SeqCst);
    }
}

#[inline(always)]
pub fn trace_validate_object<T: TransitiveClosure, F: Fn(ObjectReference)>(trace: &mut T, object: ObjectReference, validate: F) -> ObjectReference {
    if ENABLE {
        let mark_word = object.to_address() + VMObjectModel::GC_HEADER_OFFSET();
        let mark_state = MARK_STATE.load(Ordering::Relaxed);
        if unsafe { mark_word.load::<usize>() } != mark_state {
            unsafe { mark_word.store(mark_state) };
            validate(object);
            trace.process_node(object);
        }
    }
    return object;
}