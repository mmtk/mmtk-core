use std::sync::Mutex;
use std::cell::UnsafeCell;
use std::vec::Vec;

use ::util::OpaquePointer;
use ::util::{Address, ObjectReference};
use ::vm::{ActivePlan, ReferenceGlue};
use ::plan::{Plan, TraceLocal, MutatorContext};
use ::plan::selected_plan::SelectedPlan;

use libc::c_void;
use vm::VMBinding;

pub struct ReferenceProcessors {
    soft: ReferenceProcessor,
    weak: ReferenceProcessor,
    phantom: ReferenceProcessor,
}

impl ReferenceProcessors {
    pub fn new() -> Self {
        ReferenceProcessors {
            soft: ReferenceProcessor::new(Semantics::SOFT),
            weak: ReferenceProcessor::new(Semantics::WEAK),
            phantom: ReferenceProcessor::new(Semantics::PHANTOM),
        }
    }

    pub fn get(&self, semantics: Semantics) -> &ReferenceProcessor {
        match semantics {
            Semantics::SOFT => &self.soft,
            Semantics::WEAK => &self.weak,
            Semantics::PHANTOM => &self.phantom,
        }
    }

    pub fn add_soft_candidate<VM: VMBinding>(&self, reff: ObjectReference, referent: ObjectReference) {
        self.soft.add_candidate::<VM>(reff, referent);
    }

    pub fn add_weak_candidate<VM: VMBinding>(&self, reff: ObjectReference, referent: ObjectReference) {
        self.weak.add_candidate::<VM>(reff, referent);
    }

    pub fn add_phantom_candidate<VM: VMBinding>(&self, reff: ObjectReference, referent: ObjectReference) {
        self.phantom.add_candidate::<VM>(reff, referent);
    }

    pub fn forward_refs<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T) {
        self.soft.forward::<VM, T>(trace, false);
        self.weak.forward::<VM, T>(trace, false);
        self.phantom.forward::<VM, T>(trace, false);
    }

    pub fn scan_weak_refs<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, tls: OpaquePointer) {
        self.soft.scan::<VM, T>(trace, false, false, tls);
        self.weak.scan::<VM, T>(trace, false, false, tls);
    }

    pub fn scan_soft_refs<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, tls: OpaquePointer) {
        self.soft.scan::<VM, T>(trace, false, false, tls);
    }

    pub fn scan_phantom_refs<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, tls: OpaquePointer) {
        self.phantom.scan::<VM, T>(trace, false, false, tls);
    }
}

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
    sync: UnsafeCell<Mutex<ReferenceProcessorSync>>,

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

struct ReferenceProcessorSync {
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
    pub fn new(semantics: Semantics) -> Self {
        ReferenceProcessor {
            sync: UnsafeCell::new(Mutex::new(ReferenceProcessorSync {
                references: Vec::with_capacity(INITIAL_SIZE),
                unforwarded_references: None,
                nursery_index: 0,
            })),
            semantics,
        }
    }

    fn sync(&self) -> &Mutex<ReferenceProcessorSync> {
        unsafe {
            &*self.sync.get()
        }
    }

    // UNSAFE: Bypasses mutex
    unsafe fn sync_mut(&self) -> &mut ReferenceProcessorSync {
        (&mut *self.sync.get()).get_mut().unwrap()
    }

    pub fn add_candidate<VM: VMBinding>(&self, reff: ObjectReference, referent: ObjectReference) {
        let mut sync = self.sync().lock().unwrap();
        VM::VMReferenceGlue::set_referent(reff, referent);
        sync.references.push(reff.to_address());
    }

    pub fn forward<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, nursery: bool) {
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
            VM::VMReferenceGlue::set_referent(reference, trace.get_forwarded_referent(
                VM::VMReferenceGlue::get_referent(reference)));
            let new_reference = trace.get_forwarded_reference(reference);
            unforwarded_references[i] = new_reference.to_address();
        }

        if TRACE { trace!("Ending ReferenceProcessor.forward({:?})", self.semantics) }
        sync.unforwarded_references = None;
    }

    fn scan<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, nursery: bool, retain: bool, tls: OpaquePointer) {
        let sync = unsafe { self.sync_mut() };
        sync.unforwarded_references = Some(sync.references.clone());
        let references: &mut Vec<Address> = &mut sync.references;

        if TRACE { trace!("Starting ReferenceProcessor.scan({:?})", self.semantics); }
        let mut to_index = if nursery { sync.nursery_index } else { 0 };

        if TRACE_DETAIL { trace!("{:?} Reference table is {:?}", self.semantics, references); }
        if retain {
            for from_index in to_index .. references.len() {
                let reference = unsafe { references[from_index].to_object_reference() };
                self.retain_referent::<VM, T>(trace, reference);
            }
        } else {
            for from_index in to_index .. references.len() {
                let reference = unsafe { references[from_index].to_object_reference() };

                /* Determine liveness (and forward if necessary) the reference */
                let new_reference = VM::VMReferenceGlue::process_reference(trace, reference, tls);
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
            trace!("{:?} references: {} -> {}", self.semantics, references.len(), to_index);
            sync.nursery_index = to_index;
            references.truncate(to_index);
        }

        /* flush out any remset entries generated during the above activities */
        unsafe { VM::VMActivePlan::mutator(tls).flush_remembered_sets(); }
        if TRACE { trace!("Ending ReferenceProcessor.scan({:?})", self.semantics); }
    }

    /**
     * This method deals only with soft references. It retains the referent
     * if the reference is definitely reachable.
     * @param reference the address of the reference. This may or may not
     * be the address of a heap object, depending on the VM.
     * @param trace the thread local trace element.
     */
    fn retain_referent<VM: VMBinding, T: TraceLocal>(&self, trace: &mut T, reference: ObjectReference) {
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
        let referent = VM::VMReferenceGlue::get_referent(reference);
        if !referent.is_null() {
            trace.retain_referent(referent);
        }
        if TRACE_DETAIL { trace!(" ~> {:?} (retained)", referent.to_address()); }
    }
}
