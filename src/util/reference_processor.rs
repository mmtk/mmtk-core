use std::cell::UnsafeCell;
use std::sync::Mutex;
use std::vec::Vec;

// use crate::plan::TraceLocal;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::ReferenceGlue;
use crate::vm::VMBinding;
use crate::scheduler::ProcessEdgesWork;

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

    pub fn clear(&self) {
        self.soft.clear();
        self.weak.clear();
        self.phantom.clear();
    }

    pub fn add_soft_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add soft candidate: {} -> {}", reff, referent);
        self.soft.add_candidate::<VM>(reff, referent);
    }

    pub fn add_weak_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add weak candidate: {} -> {}", reff, referent);
        self.weak.add_candidate::<VM>(reff, referent);
    }

    pub fn add_phantom_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add phantom candidate: {} -> {}", reff, referent);
        self.phantom.add_candidate::<VM>(reff, referent);
    }

    pub fn enqueue_refs<E: ProcessEdgesWork>(&self, trace: &mut E) {
        self.soft.enqueue::<E>(trace, false);
        self.weak.enqueue::<E>(trace, false);
        self.phantom.enqueue::<E>(trace, false);
    }

    pub fn forward_refs<E: ProcessEdgesWork>(&self, trace: &mut E) {
        self.soft.forward::<E>(trace, false);
        self.weak.forward::<E>(trace, false);
        self.phantom.forward::<E>(trace, false);
    }

    pub fn scan_weak_refs<E: ProcessEdgesWork>(&self, trace: &mut E, tls: VMWorkerThread) {
        self.soft.scan::<E>(trace, false, false, tls);
        self.weak.scan::<E>(trace, false, false, tls);
    }

    pub fn scan_soft_refs<E: ProcessEdgesWork>(&self, trace: &mut E, tls: VMWorkerThread) {
        self.soft.scan::<E>(trace, false, false, tls);
    }

    pub fn scan_phantom_refs<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        tls: VMWorkerThread,
    ) {
        self.phantom.scan::<E>(trace, false, false, tls);
    }
}

impl Default for ReferenceProcessors {
    fn default() -> Self {
        Self::new()
    }
}

// Debug flags
pub const TRACE: bool = true;
pub const TRACE_UNREACHABLE: bool = true;
pub const TRACE_DETAIL: bool = true;
pub const TRACE_FORWARD: bool = true;

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

// TODO: We should carefully examine the unsync with UnsafeCell. We should be able to provide a safe implementation.
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
    references: Vec<ObjectReference>,

    /**
     * In a MarkCompact (or similar) collector, we need to update the {@code references}
     * field, and then update its contents.  We implement this by saving the pointer in
     * this untraced field for use during the {@code forward} pass.
     */
    // unforwarded_references: Option<Vec<Address>>,

    enqueued_references: Vec<ObjectReference>,

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
                // unforwarded_references: None,
                enqueued_references: vec![],
                nursery_index: 0,
            })),
            semantics,
        }
    }

    fn sync(&self) -> &Mutex<ReferenceProcessorSync> {
        unsafe { &*self.sync.get() }
    }

    // UNSAFE: Bypasses mutex
    // It is designed to allow getting mut ref from UnsafeCell.
    // TODO: We may need to rework on this to remove the unsafety.
    #[allow(clippy::mut_from_ref)]
    unsafe fn sync_mut(&self) -> &mut ReferenceProcessorSync {
        (*self.sync.get()).get_mut().unwrap()
    }

    pub fn clear(&self) {
        let mut sync = self.sync().lock().unwrap();
        sync.references.clear();
        // sync.unforwarded_references = None;
        sync.nursery_index = 0;
    }

    pub fn add_candidate<VM: VMBinding>(&self, reff: ObjectReference, referent: ObjectReference) {
        let mut sync = self.sync().lock().unwrap();
        // VM::VMReferenceGlue::set_referent(reff, referent);
        if sync.references.iter().any(|r| *r == reff) {
            return;
        }
        sync.references.push(reff);
    }

    fn get_forwarded_referent<E: ProcessEdgesWork>(e: &mut E, object: ObjectReference) -> ObjectReference {
        e.trace_object(object)
    }

    fn get_forwarded_reference<E: ProcessEdgesWork>(e: &mut E, object: ObjectReference) -> ObjectReference {
        e.trace_object(object)
    }

    fn retain_referent<E: ProcessEdgesWork>(e: &mut E, object: ObjectReference) -> ObjectReference {
        e.trace_object(object)
    }

    pub fn enqueue<E: ProcessEdgesWork>(&self, trace: &mut E, _nursery: bool) {
        let mut sync = unsafe { self.sync_mut() };

        if !sync.enqueued_references.is_empty() {
            debug!("enqueue: {:?}", sync.enqueued_references);
            <E::VM as VMBinding>::VMReferenceGlue::enqueue_references(&sync.enqueued_references);
            sync.enqueued_references.clear();
        }
    }

    pub fn forward<E: ProcessEdgesWork>(&self, trace: &mut E, _nursery: bool) {
        let mut sync = unsafe { self.sync_mut() };
        let references: &mut Vec<ObjectReference> = &mut sync.references;
        // XXX: Copies `unforwarded_references` out. Should be fine since it's not accessed
        //      concurrently & it's set to `None` at the end anyway..
        // let mut unforwarded_references: Vec<Address> = sync.unforwarded_references.clone().unwrap();
        if TRACE {
            trace!("Starting ReferenceProcessor.forward({:?})", self.semantics);
        }
        // if TRACE_DETAIL {
        //     trace!("{:?} Reference table is {:?}", self.semantics, references);
        //     trace!(
        //         "{:?} unforwardedReferences is {:?}",
        //         self.semantics,
        //         unforwarded_references
        //     );
        // }

        // for (i, unforwarded_ref) in unforwarded_references
        //     .iter_mut()
        //     .enumerate()
        //     .take(references.len())
        // {
        //     let reference = unsafe { unforwarded_ref.to_object_reference() };
        //     if TRACE_DETAIL {
        //         trace!("slot {:?}: forwarding {:?}", i, reference);
        //     }
        //     <E::VM as VMBinding>::VMReferenceGlue::set_referent(
        //         reference,
        //         Self::get_forwarded_referent(trace, <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference)),
        //     );
        //     let new_reference = Self::get_forwarded_reference(trace, reference);
        //     *unforwarded_ref = new_reference.to_address();
        // }

        let mut new_queue = vec![];
        for obj in sync.references.drain(..) {
            let old_referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(obj);
            let new_referent = Self::get_forwarded_referent(trace, old_referent);
            if !old_referent.is_null() {
                // make sure MC forwards the referent
                // debug_assert!(old_referent != new_referent);
            }
            <E::VM as VMBinding>::VMReferenceGlue::set_referent(
                obj,
                new_referent,
            );
            let new_reference = Self::get_forwarded_reference(trace, obj);
            // debug_assert!(!obj.is_null());
            // debug_assert!(obj != new_reference);
            if TRACE_DETAIL {
                use crate::vm::ObjectModel;
                trace!("Forwarding reference: {} (size: {})", obj, <E::VM as VMBinding>::VMObjectModel::get_current_size(obj));
                trace!(" referent: {} (forwarded to {})", old_referent, new_referent);
                trace!(" reference: forwarded to {}", new_reference);
            }
            // <E::VM as VMBinding>::VMReferenceGlue::enqueue_reference(new_reference);
            debug_assert!(!new_reference.is_null(), "reference {:?}'s forwarding pointer is NULL", obj);
            new_queue.push(new_reference);
        }
        sync.references = new_queue;

        let mut new_queue = vec![];
        for obj in sync.enqueued_references.drain(..) {
            let old_referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(obj);
            let new_referent = Self::get_forwarded_referent(trace, old_referent);
            if !old_referent.is_null() {
                // make sure MC forwards the referent
                // debug_assert!(old_referent != new_referent);
            }
            <E::VM as VMBinding>::VMReferenceGlue::set_referent(
                obj,
                new_referent,
            );
            let new_reference = Self::get_forwarded_reference(trace, obj);
            // debug_assert!(!obj.is_null());
            // debug_assert!(obj != new_reference);
            if TRACE_DETAIL {
                use crate::vm::ObjectModel;
                trace!("Forwarding enqueued reference: {} (size: {})", obj, <E::VM as VMBinding>::VMObjectModel::get_current_size(obj));
                trace!(" referent: {} (forwarded to {})", old_referent, new_referent);
                trace!(" reference: forwarded to {}", new_reference);
            }
            // <E::VM as VMBinding>::VMReferenceGlue::enqueue_reference(new_reference);
            debug_assert!(!new_reference.is_null(), "reference {:?}'s forwarding pointer is NULL", obj);
            new_queue.push(new_reference);
        }
        sync.enqueued_references = new_queue;

        if TRACE {
            trace!("Ending ReferenceProcessor.forward({:?})", self.semantics)
        }
        // sync.unforwarded_references = None;
    }

    fn scan<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        nursery: bool,
        retain: bool,
        tls: VMWorkerThread,
    ) {
        let sync = unsafe { self.sync_mut() };
        // sync.unforwarded_references = Some(sync.references.clone());
        let references: &mut Vec<ObjectReference> = &mut sync.references;

        if TRACE {
            trace!("Starting ReferenceProcessor.scan({:?})", self.semantics);
        }
        let mut to_index = if nursery { sync.nursery_index } else { 0 };
        let from_index = to_index;

        if TRACE_DETAIL {
            trace!("{:?} Reference table is {:?}", self.semantics, references);
        }
        if retain {
            for reference in references.iter().skip(from_index) {
                self.scan_retain_referent::<E>(trace, *reference);
            }
        } else {
            for i in from_index..references.len() {
                let reference = references[i];

                /* Determine liveness (and forward if necessary) the reference */
                let new_reference = self.process_reference(trace, reference, tls, &mut sync.enqueued_references);
                if !new_reference.is_null() {
                    references[to_index] = new_reference;
                    to_index += 1;
                    if TRACE_DETAIL {
                        let index = to_index - 1;
                        trace!(
                            "SCANNED {} {:?}",
                            index,
                            references[index],
                        );
                    }
                }
            }
            trace!(
                "{:?} references: {} -> {}",
                self.semantics,
                references.len(),
                to_index
            );
            sync.nursery_index = to_index;
            references.truncate(to_index);
        }

        /* flush out any remset entries generated during the above activities */

        // FIXME: We are calling mutator() for a worker thread
        // panic!("We are calling mutator() for a worker tls. We need to fix this.");
        // unsafe { VM::VMActivePlan::mutator(tls)) }.flush_remembered_sets();
        // if TRACE {
        //     trace!("Ending ReferenceProcessor.scan({:?})", self.semantics);
        // }
    }

    /**
     * This method deals only with soft references. It retains the referent
     * if the reference is definitely reachable.
     * @param reference the address of the reference. This may or may not
     * be the address of a heap object, depending on the VM.
     * @param trace the thread local trace element.
     */
    fn scan_retain_referent<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        reference: ObjectReference,
    ) {
        debug_assert!(!reference.is_null());
        debug_assert!(self.semantics == Semantics::SOFT);

        if TRACE_DETAIL {
            trace!("Processing reference: {:?}", reference);
        }

        if !reference.is_live() {
            /*
             * Reference is currently unreachable but may get reachable by the
             * following trace. We postpone the decision.
             */
            return;
        }

        /*
         * Reference is definitely reachable.  Retain the referent.
         */
        let referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference);
        if !referent.is_null() {
            Self::retain_referent(trace, referent);
        }
        if TRACE_DETAIL {
            trace!(" ~> {:?} (retained)", referent.to_address());
        }
    }

    /// Process a reference with the current semantics and return an updated reference (e.g. with a new address)
    /// if the reference is still alive, otherwise return a null object reference.
    ///
    /// Arguments:
    /// * `trace`: A reference to a `TraceLocal` object for this reference.
    /// * `reference`: The address of the reference. This may or may not be the address of a heap object, depending on the VM.
    /// * `tls`: The GC thread that is processing this reference.
    fn process_reference<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        reference: ObjectReference,
        tls: VMWorkerThread,
        enqueued_references: &mut Vec<ObjectReference>,
    ) -> ObjectReference {
        debug_assert!(!reference.is_null());

        if TRACE_DETAIL { trace!("Process reference: {}", reference); }

        // If the reference is dead, we're done with it. Let it (and
        // possibly its referent) be garbage-collected.
        if !reference.is_live() {
            <E::VM as VMBinding>::VMReferenceGlue::clear_referent(reference);
            if TRACE_UNREACHABLE { trace!(" UNREACHABLE reference: {}", reference); }
            if TRACE_DETAIL { trace!(" (unreachable)"); }
            return ObjectReference::NULL;
        }

        // The reference object is live
        let new_reference = Self::get_forwarded_reference(trace, reference);
        // This assert should be true for mark compact
        // debug_assert_eq!(reference, new_reference);
        let old_referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference);

        if TRACE_DETAIL { trace!(" ~> {}", old_referent); }

        // If the application has cleared the referent the Java spec says
        // this does not cause the Reference object to be enqueued. We
        // simply allow the Reference object to fall out of our
        // waiting list.
        if old_referent.is_null() {
            if TRACE_DETAIL { trace!(" (null referent) "); }
            return ObjectReference::NULL;
        }

        if TRACE_DETAIL { trace!(" => {}", new_reference); }

        if old_referent.is_live() {
            // Referent is still reachable in a way that is as strong as
            // or stronger than the current reference level.
            let new_referent = Self::get_forwarded_referent(trace, old_referent);
            // This assert should be true for mark compact
            // debug_assert_eq!(old_referent, new_referent);

            if TRACE_DETAIL { trace!(" ~> {}", new_referent); }
            debug_assert!(new_referent.is_live());

            // The reference object stays on the waiting list, and the
            // referent is untouched. The only thing we must do is
            // ensure that the former addresses are updated with the
            // new forwarding addresses in case the collector is a
            // copying collector.

            // Update the referent
            <E::VM as VMBinding>::VMReferenceGlue::set_referent(new_reference, new_referent);
            return new_reference;
        } else {
            // Referent is unreachable. Clear the referent and enqueue the reference object.
            if TRACE_DETAIL { trace!(" UNREACHABLE"); }
            else if TRACE_UNREACHABLE { trace!(" UNREACHABLE referent: {}", old_referent); }

            <E::VM as VMBinding>::VMReferenceGlue::clear_referent(new_reference);
            // <E::VM as VMBinding>::VMReferenceGlue::enqueue_reference(new_reference);
            enqueued_references.push(new_reference);
            return ObjectReference::NULL;
        }
    }
}

use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::MMTK;
use std::marker::PhantomData;

pub struct SoftRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for SoftRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_soft_refs(&mut w, worker.tls);
    }
}
impl<E: ProcessEdgesWork> SoftRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct WeakRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for WeakRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_weak_refs(&mut w, worker.tls);
    }
}
impl<E: ProcessEdgesWork> WeakRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct PhantomRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for PhantomRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_phantom_refs(&mut w, worker.tls);
    }
}
impl<E: ProcessEdgesWork> PhantomRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct RefForwarding<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for RefForwarding<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.forward_refs(&mut w);
    }
}
impl<E: ProcessEdgesWork> RefForwarding<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct RefEnqueue<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for RefEnqueue<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.enqueue_refs(&mut w);
    }
}
impl<E: ProcessEdgesWork> RefEnqueue<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
