use std::sync::Mutex;
use std::cell::UnsafeCell;
use std::vec::Vec;

use ::util::{Address, ObjectReference};
use ::util::options::OPTION_MAP;
use ::vm::{ActivePlan, VMActivePlan, ReferenceGlue, VMReferenceGlue};
use ::plan::{Plan, TraceLocal, MutatorContext};
use ::plan::selected_plan::SelectedPlan;

// Debug flags
pub const TRACE: bool = false;
pub const TRACE_UNREACHABLE: bool = false;
pub const TRACE_DETAIL: bool = false;
pub const TRACE_FORWARD: bool = false;

// XXX: We differ from the original implementation
//      by ignoring "stress," i.e. where the array
//      of references is grown by 1 each time. We
//      can't do this here b/c std::vec::Vec doesn't
//      allow us to customize its behaviour like that.
//      (Similarly, GROWTH_FACTOR is locked at 2.0, but
//      luckily this is also the value used by Java MMTk.)
const INITIAL_SIZE: usize = 256;

pub struct ReferenceProcessor {
    // XXX: To support the possibility of the collector working
    //      on the reference in parallel, we wrap the structure
    //      in an UnsafeCell.
    sync: UnsafeCell<Mutex<ReferenceprocessorSync>>,

    /**
     * Semantics
     */
    semantics: Semantics,
}

unsafe impl Sync for ReferenceProcessor {}

#[derive(Debug, PartialEq)]
pub enum Semantics {
    SOFT,
    WEAK,
    PHANTOM,
}

lazy_static! {
    static ref SOFT_REFERENCE_PROCESSOR: ReferenceProcessor = ReferenceProcessor::new(Semantics::SOFT);
    static ref WEAK_REFERENCE_PROCESSOR: ReferenceProcessor = ReferenceProcessor::new(Semantics::WEAK);
    static ref PHANTOM_REFERENCE_PROCESSOR: ReferenceProcessor = ReferenceProcessor::new(Semantics::PHANTOM);
}

struct ReferenceprocessorSync {
    // XXX: A data race on any of these fields is UB. If
    //      parallelizing this code, change the types to
    //      have the correct semantics.
    /**
     * The table of reference objects for the current semantics
     */
    references: Vec<Address>,

    /**
     * In a MarkCompact (or similar) collector, we need to update the {@code references}
     * field, and then update its contents.  We implement this by saving the pointer in
     * this untraced field for use during the {@code forward} pass.
     */
    unforwarded_references: Option<Vec<Address>>,

    /**
     * Index into the <code>references</code> table for the start of
     * the reference nursery.
     */
    nursery_index: usize,
}

impl ReferenceProcessor {
    fn new(semantics: Semantics) -> Self {
        ReferenceProcessor {
            sync: UnsafeCell::new(Mutex::new(ReferenceprocessorSync {
                references: Vec::with_capacity(INITIAL_SIZE),
                unforwarded_references: None,
                nursery_index: 0,
            })),
            semantics,
        }
    }

    fn sync(&self) -> &Mutex<ReferenceprocessorSync> {
        unsafe {
            &*self.sync.get()
        }
    }

    // UNSAFE: Bypasses mutex
    unsafe fn sync_mut(&self) -> &mut ReferenceprocessorSync {
        (&mut *self.sync.get()).get_mut().unwrap()
    }

    pub fn get(semantics: Semantics) -> &'static Self {
        match semantics {
            Semantics::SOFT => &SOFT_REFERENCE_PROCESSOR,
            Semantics::WEAK => &WEAK_REFERENCE_PROCESSOR,
            Semantics::PHANTOM => &PHANTOM_REFERENCE_PROCESSOR,
        }
    }

    fn add_candidate(&self, reff: ObjectReference, referent: ObjectReference) {
        let mut sync = self.sync().lock().unwrap();
        VMReferenceGlue::set_referent(reff, referent);
        sync.references.push(reff.to_address());
    }

    fn forward<T: TraceLocal>(&self, trace: &mut T, nursery: bool) {
        let mut sync = unsafe { self.sync_mut() };
        let references: &mut Vec<Address> = &mut sync.references;
        // XXX: Copies `unforwarded_references` out. Should be fine since it's not accessed
        //      concurrently & it's set to `None` at the end anyway..
        let mut unforwarded_references: Vec<Address> = sync.unforwarded_references.clone().unwrap();
        if TRACE { trace!("Starting ReferenceProcessor.forward({:?})", self.semantics); }
        if TRACE_DETAIL {
            trace!("{:?} Reference table is {:?}", self.semantics, references);
            trace!("{:?} unforwardedReferences is {:?}", self.semantics, unforwarded_references);
        }

        for i in 0 .. references.len() {
            let reference = unsafe { unforwarded_references[i].to_object_reference() };
            if TRACE_DETAIL { trace!("slot {:?}: forwarding {:?}", i, reference); }
            VMReferenceGlue::set_referent(reference, trace.get_forwarded_referent(
                VMReferenceGlue::get_referent(reference)));
            let new_reference = trace.get_forwarded_reference(reference);
            unforwarded_references[i] = new_reference.to_address();
        }

        if TRACE { trace!("Ending ReferenceProcessor.forward({:?})", self.semantics) }
        sync.unforwarded_references = None;
    }

    fn scan<T: TraceLocal>(&self, trace: &mut T, nursery: bool, retain: bool, thread_id: usize) {
        let sync = unsafe { self.sync_mut() };
        sync.unforwarded_references = Some(sync.references.clone());
        let references: &mut Vec<Address> = &mut sync.references;

        if TRACE { trace!("Starting ReferenceProcessor.scan({:?})", self.semantics); }
        let mut to_index = if nursery { sync.nursery_index } else { 0 };

        if TRACE_DETAIL { trace!("{:?} Reference table is {:?}", self.semantics, references); }
        if retain {
            for from_index in to_index .. references.len() {
                let reference = unsafe { references[from_index].to_object_reference() };
                self.retain_referent(trace, reference);
            }
        } else {
            for from_index in to_index .. references.len() {
                let reference = unsafe { references[from_index].to_object_reference() };

                /* Determine liveness (and forward if necessary) the reference */
                let new_reference = VMReferenceGlue::process_reference(trace, reference, thread_id);
                if !new_reference.is_null() {
                    references[to_index] = new_reference.to_address();
                    to_index += 1;
                    if TRACE_DETAIL {
                        let index = to_index - 1;
                        trace!("SCANNED {} {:?} -> {:?}", index, references[index],
                               unsafe { references[index].to_object_reference() });
                    }
                }
            }
            if OPTION_MAP.verbose >= 3 {
                trace!("{:?} references: {} -> {}", self.semantics, references.len(), to_index);
            }
            sync.nursery_index = to_index;
        }

        /* flush out any remset entries generated during the above activities */
        unsafe { VMActivePlan::mutator(thread_id).flush_remembered_sets(); }
        if TRACE { trace!("Ending ReferenceGlue.scan({:?})", self.semantics); }
    }

    /**
     * This method deals only with soft references. It retains the referent
     * if the reference is definitely reachable.
     * @param reference the address of the reference. This may or may not
     * be the address of a heap object, depending on the VM.
     * @param trace the thread local trace element.
     */
    fn retain_referent<T: TraceLocal>(&self, trace: &mut T, reference: ObjectReference) {
        debug_assert!(!reference.is_null());
        debug_assert!(self.semantics == Semantics::SOFT);

        if TRACE_DETAIL { trace!("Processing reference: {:?}", reference); }

        if !trace.is_live(reference) {
            /*
             * Reference is currently unreachable but may get reachable by the
             * following trace. We postpone the decision.
             */
            return;
        }

        /*
         * Reference is definitely reachable.  Retain the referent.
         */
        let referent = VMReferenceGlue::get_referent(reference);
        if !referent.is_null() {
            trace.retain_referent(referent);
        }
        if TRACE_DETAIL { trace!(" ~> {:?} (retained)", referent.to_address()); }
    }
}

pub fn add_soft_candidate(reff: ObjectReference, referent: ObjectReference) {
    SOFT_REFERENCE_PROCESSOR.add_candidate(reff, referent);
}

pub fn add_weak_candidate(reff: ObjectReference, referent: ObjectReference) {
    WEAK_REFERENCE_PROCESSOR.add_candidate(reff, referent);
}

pub fn add_phantom_candidate(reff: ObjectReference, referent: ObjectReference) {
    PHANTOM_REFERENCE_PROCESSOR.add_candidate(reff, referent);
}

pub fn forward_refs<T: TraceLocal>(trace: &mut T) {
    SOFT_REFERENCE_PROCESSOR.forward(trace, false);
    WEAK_REFERENCE_PROCESSOR.forward(trace, false);
    PHANTOM_REFERENCE_PROCESSOR.forward(trace, false);
}

pub fn scan_weak_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    SOFT_REFERENCE_PROCESSOR.scan(trace, false, false, thread_id);
    WEAK_REFERENCE_PROCESSOR.scan(trace, false, false, thread_id);
}

pub fn scan_soft_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    SOFT_REFERENCE_PROCESSOR.scan(trace, false, false, thread_id);
}

pub fn scan_phantom_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    PHANTOM_REFERENCE_PROCESSOR.scan(trace, false, false, thread_id);
}