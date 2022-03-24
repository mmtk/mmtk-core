use std::sync::Mutex;
use std::vec::Vec;

use crate::scheduler::ProcessEdgesWork;
use crate::util::ObjectReference;
use crate::vm::ReferenceGlue;
use crate::vm::VMBinding;

/// Holds all reference processors for each weak reference Semantics.
/// Currently this is based on Java's weak reference semantics (soft/weak/phantom).
/// We should make changes to make this general rather than Java specific.
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

    pub fn add_soft_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add soft candidate: {} ~> {}", reff, referent);
        self.soft.add_candidate::<VM>(reff, referent);
    }

    pub fn add_weak_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add weak candidate: {} ~> {}", reff, referent);
        self.weak.add_candidate::<VM>(reff, referent);
    }

    pub fn add_phantom_candidate<VM: VMBinding>(
        &self,
        reff: ObjectReference,
        referent: ObjectReference,
    ) {
        trace!("Add phantom candidate: {} ~> {}", reff, referent);
        self.phantom.add_candidate::<VM>(reff, referent);
    }

    /// This will invoke enqueue for each reference processor, which will
    /// call back to the VM to enqueue references whose referents are cleared
    /// in this GC.
    pub fn enqueue_refs<VM: VMBinding>(&self) {
        self.soft.enqueue::<VM>();
        self.weak.enqueue::<VM>();
        self.phantom.enqueue::<VM>();
    }

    /// A separate reference forwarding step. Normally when we scan refs, we deal with forwarding.
    /// However, for some plans like mark compact, at the point we do ref scanning, we do not know
    /// the forwarding addresses yet, thus we cannot do forwarding during scan refs. And for those
    /// plans, this separate step is required.
    pub fn forward_refs<E: ProcessEdgesWork>(&self, trace: &mut E, mmtk: &'static MMTK<E::VM>) {
        debug_assert!(
            mmtk.plan.constraints().needs_forward_after_liveness,
            "A plan with needs_forward_after_liveness=false does not need a separate forward step"
        );
        self.soft
            .forward::<E>(trace, mmtk.plan.is_current_gc_nursery());
        self.weak
            .forward::<E>(trace, mmtk.plan.is_current_gc_nursery());
        self.phantom
            .forward::<E>(trace, mmtk.plan.is_current_gc_nursery());
    }

    // Methods for scanning weak references. It needs to be called in a decreasing order of reference strengths, i.e. soft > weak > phantom

    /// Scan weak references.
    pub fn scan_weak_refs<E: ProcessEdgesWork>(&self, trace: &mut E, mmtk: &'static MMTK<E::VM>) {
        self.soft
            .scan::<E>(trace, mmtk.plan.is_current_gc_nursery(), false);
        self.weak
            .scan::<E>(trace, mmtk.plan.is_current_gc_nursery(), false);
    }

    /// Scan soft references.
    pub fn scan_soft_refs<E: ProcessEdgesWork>(&self, trace: &mut E, mmtk: &'static MMTK<E::VM>) {
        // For soft refs, it is up to the VM to decide when to reclaim this.
        // If this is not an emergency collection, we have no heap stress. We simply retain soft refs.
        if !mmtk.plan.is_emergency_collection() {
            // This step only retains the referents (keep the referents alive), it does not update its addresses.
            // We will call soft.scan() again with retain=false to update its addresses based on liveness.
            self.soft
                .scan::<E>(trace, mmtk.plan.is_current_gc_nursery(), true);
        }
    }

    /// Scan phantom references.
    pub fn scan_phantom_refs<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        mmtk: &'static MMTK<E::VM>,
    ) {
        self.phantom
            .scan::<E>(trace, mmtk.plan.is_current_gc_nursery(), false);
    }
}

impl Default for ReferenceProcessors {
    fn default() -> Self {
        Self::new()
    }
}

// XXX: We differ from the original implementation
//      by ignoring "stress," i.e. where the array
//      of references is grown by 1 each time. We
//      can't do this here b/c std::vec::Vec doesn't
//      allow us to customize its behaviour like that.
//      (Similarly, GROWTH_FACTOR is locked at 2.0, but
//      luckily this is also the value used by Java MMTk.)
const INITIAL_SIZE: usize = 256;

/// We create a reference processor for each semantics. Generally we expect these
/// to happen for each processor:
/// 1. The VM adds reference candidates. They could either do it when a weak reference
///    is created, or when a weak reference is traced during GC.
/// 2. We scan references after the GC determins liveness.
/// 3. We forward references if the GC needs forwarding after liveness.
/// 4. We inform the binding of references whose referents are cleared during this GC by enqueue'ing.
pub struct ReferenceProcessor {
    /// Most of the reference processor is protected by a mutex.
    sync: Mutex<ReferenceProcessorSync>,

    /// The semantics for the reference processor
    semantics: Semantics,
}

#[derive(Debug, PartialEq)]
pub enum Semantics {
    SOFT,
    WEAK,
    PHANTOM,
}

struct ReferenceProcessorSync {
    /// The table of reference objects for the current semantics. We add references to this table by
    /// add_candidate(). After scanning this table, a reference in the table should either
    /// stay in the table (if the referent is alive) or go to enqueued_reference (if the referent is dead and cleared).
    /// Note that this table should not have duplicate entries, otherwise we will scan the duplicates multiple times, and
    /// that may lead to incorrect results.
    references: Vec<ObjectReference>,

    /// References whose referents are cleared during this GC. We add references to this table during
    /// scanning, and we pop from this table during the enqueue work at the end of GC.
    enqueued_references: Vec<ObjectReference>,

    /// Index into the references table for the start of nursery objects
    nursery_index: usize,
}

impl ReferenceProcessor {
    pub fn new(semantics: Semantics) -> Self {
        ReferenceProcessor {
            sync: Mutex::new(ReferenceProcessorSync {
                references: Vec::with_capacity(INITIAL_SIZE),
                enqueued_references: vec![],
                nursery_index: 0,
            }),
            semantics,
        }
    }

    /// Add a candidate.
    // TODO: do we need the referent argument?
    pub fn add_candidate<VM: VMBinding>(&self, reff: ObjectReference, _referent: ObjectReference) {
        let mut sync = self.sync.lock().unwrap();
        // We make sure that we do not have duplicate entries
        // TODO: Should we use hash set instead?
        if sync.references.iter().any(|r| *r == reff) {
            return;
        }
        sync.references.push(reff);
    }

    // These funcions simply call `trace_object()`, which does two things: 1. to make sure the object is kept alive
    // and 2. to get the new object reference if the object is copied. The functions are intended to make the code
    // easier to understand.

    #[inline(always)]
    fn get_forwarded_referent<E: ProcessEdgesWork>(
        e: &mut E,
        referent: ObjectReference,
    ) -> ObjectReference {
        e.trace_object(referent)
    }

    #[inline(always)]
    fn get_forwarded_reference<E: ProcessEdgesWork>(
        e: &mut E,
        object: ObjectReference,
    ) -> ObjectReference {
        e.trace_object(object)
    }

    #[inline(always)]
    fn keep_referent_alive<E: ProcessEdgesWork>(
        e: &mut E,
        referent: ObjectReference,
    ) -> ObjectReference {
        e.trace_object(referent)
    }

    /// Inform the binding to enqueue the weak references whose referents were cleared in this GC.
    pub fn enqueue<VM: VMBinding>(&self) {
        let mut sync = self.sync.lock().unwrap();

        if !sync.enqueued_references.is_empty() {
            debug!("enqueue: {:?}", sync.enqueued_references);
            VM::VMReferenceGlue::enqueue_references(&sync.enqueued_references);
            sync.enqueued_references.clear();
        }
    }

    /// Forward the reference tables in the reference processor. This is only needed if a plan does not forward
    /// objects in their first transitive closure.
    /// nursery is not used for this.
    pub fn forward<E: ProcessEdgesWork>(&self, trace: &mut E, _nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        debug!("Starting ReferenceProcessor.forward({:?})", self.semantics);

        // Forward a single reference
        #[inline(always)]
        fn forward_reference<E: ProcessEdgesWork>(
            trace: &mut E,
            reference: ObjectReference,
        ) -> ObjectReference {
            let old_referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference);
            let new_referent = ReferenceProcessor::get_forwarded_referent(trace, old_referent);
            <E::VM as VMBinding>::VMReferenceGlue::set_referent(reference, new_referent);
            let new_reference = ReferenceProcessor::get_forwarded_reference(trace, reference);
            {
                use crate::vm::ObjectModel;
                trace!(
                    "Forwarding reference: {} (size: {})",
                    reference,
                    <E::VM as VMBinding>::VMObjectModel::get_current_size(reference)
                );
                trace!(
                    " referent: {} (forwarded to {})",
                    old_referent,
                    new_referent
                );
                trace!(" reference: forwarded to {}", new_reference);
            }
            debug_assert!(
                !new_reference.is_null(),
                "reference {:?}'s forwarding pointer is NULL",
                reference
            );
            new_reference
        }

        sync.references
            .iter_mut()
            .for_each(|slot: &mut ObjectReference| {
                let reference = *slot;
                *slot = forward_reference::<E>(trace, reference);
            });

        sync.enqueued_references
            .iter_mut()
            .for_each(|slot: &mut ObjectReference| {
                let reference = *slot;
                *slot = forward_reference::<E>(trace, reference);
            });

        debug!("Ending ReferenceProcessor.forward({:?})", self.semantics)
    }

    /// Scan the reference table.
    fn scan<E: ProcessEdgesWork>(&self, trace: &mut E, nursery: bool, retain: bool) {
        let mut sync = self.sync.lock().unwrap();

        debug!("Starting ReferenceProcessor.scan({:?})", self.semantics);
        // Start scanning from here
        let from_index = if nursery { sync.nursery_index } else { 0 };

        debug!(
            "{:?} Reference table is {:?}",
            self.semantics, sync.references
        );
        if retain {
            for reference in sync.references.iter().skip(from_index) {
                // Retain the referent. This does not update the reference table.
                // There will be a later scan pass that update the reference table.
                self.retain_referent::<E>(trace, *reference);
            }
        } else {
            // A cursor. Live reference will be stored at the cursor.
            let mut to_index = from_index;

            // Iterate from from_index, process/forward for each reference.
            // If the reference is alive (process_reference() returned a non-NULL value),
            // store the forwarded reference back at the cursor.
            for i in from_index..sync.references.len() {
                let reference = sync.references[i];

                // Determine liveness (and forward if necessary) the reference
                let new_reference =
                    self.process_reference(trace, reference, &mut sync.enqueued_references);
                // If the reference is alive, put it back to the array
                if !new_reference.is_null() {
                    sync.references[to_index] = new_reference;
                    to_index += 1;
                    trace!("SCANNED {} {:?}", i, new_reference,);
                }
            }
            debug!(
                "{:?} references: {} -> {}",
                self.semantics,
                sync.references.len(),
                to_index
            );
            sync.nursery_index = to_index;
            sync.references.truncate(to_index);
        }

        debug!("Ending ReferenceProcessor.scan({:?})", self.semantics);
    }

    /// This method deals only with soft references. It retains the referent
    /// if the reference is definitely reachable.
    fn retain_referent<E: ProcessEdgesWork>(&self, trace: &mut E, reference: ObjectReference) {
        debug_assert!(!reference.is_null());
        debug_assert!(self.semantics == Semantics::SOFT);

        trace!("Processing reference: {:?}", reference);

        if !reference.is_live() {
            // Reference is currently unreachable but may get reachable by the
            // following trace. We postpone the decision.
            return;
        }

        // Reference is definitely reachable.  Retain the referent.
        let referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference);
        if !referent.is_null() {
            Self::keep_referent_alive(trace, referent);
        }
        trace!(" ~> {:?} (retained)", referent.to_address());
    }

    /// Process a reference.
    /// * If both the reference and the referent is alive, return the updated reference and update its referent properly.
    /// * If the reference is alive, and the referent is not null but not alive, return a null pointer and the reference (with cleared referent) is enqueued.
    /// * For other cases, return a null pointer.
    ///
    /// If a null pointer is returned, the reference can be removed from the reference table. Otherwise, the updated reference should be kept
    /// in the reference table.
    fn process_reference<E: ProcessEdgesWork>(
        &self,
        trace: &mut E,
        reference: ObjectReference,
        enqueued_references: &mut Vec<ObjectReference>,
    ) -> ObjectReference {
        debug_assert!(!reference.is_null());

        trace!("Process reference: {}", reference);

        // If the reference is dead, we're done with it. Let it (and
        // possibly its referent) be garbage-collected.
        if !reference.is_live() {
            <E::VM as VMBinding>::VMReferenceGlue::clear_referent(reference);
            trace!(" UNREACHABLE reference: {}", reference);
            trace!(" (unreachable)");
            return ObjectReference::NULL;
        }

        // The reference object is live
        let new_reference = Self::get_forwarded_reference(trace, reference);
        let old_referent = <E::VM as VMBinding>::VMReferenceGlue::get_referent(reference);
        trace!(" ~> {}", old_referent);

        // If the application has cleared the referent the Java spec says
        // this does not cause the Reference object to be enqueued. We
        // simply allow the Reference object to fall out of our
        // waiting list.
        if old_referent.is_null() {
            trace!(" (null referent) ");
            return ObjectReference::NULL;
        }

        trace!(" => {}", new_reference);

        if old_referent.is_live() {
            // Referent is still reachable in a way that is as strong as
            // or stronger than the current reference level.
            let new_referent = Self::get_forwarded_referent(trace, old_referent);
            debug_assert!(new_referent.is_live());
            trace!(" ~> {}", new_referent);

            // The reference object stays on the waiting list, and the
            // referent is untouched. The only thing we must do is
            // ensure that the former addresses are updated with the
            // new forwarding addresses in case the collector is a
            // copying collector.

            // Update the referent
            <E::VM as VMBinding>::VMReferenceGlue::set_referent(new_reference, new_referent);
            new_reference
        } else {
            // Referent is unreachable. Clear the referent and enqueue the reference object.
            trace!(" UNREACHABLE referent: {}", old_referent);

            <E::VM as VMBinding>::VMReferenceGlue::clear_referent(new_reference);
            enqueued_references.push(new_reference);
            ObjectReference::NULL
        }
    }
}

use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::MMTK;
use std::marker::PhantomData;

#[derive(Default)]
pub struct SoftRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for SoftRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_soft_refs(&mut w, mmtk);
        w.flush();
    }
}
impl<E: ProcessEdgesWork> SoftRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct WeakRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for WeakRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_weak_refs(&mut w, mmtk);
        w.flush();
    }
}
impl<E: ProcessEdgesWork> WeakRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct PhantomRefProcessing<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for PhantomRefProcessing<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.scan_phantom_refs(&mut w, mmtk);
        w.flush();
    }
}
impl<E: ProcessEdgesWork> PhantomRefProcessing<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct RefForwarding<E: ProcessEdgesWork>(PhantomData<E>);
impl<E: ProcessEdgesWork> GCWork<E::VM> for RefForwarding<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        mmtk.reference_processors.forward_refs(&mut w, mmtk);
        w.flush();
    }
}
impl<E: ProcessEdgesWork> RefForwarding<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct RefEnqueue<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> GCWork<VM> for RefEnqueue<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        mmtk.reference_processors.enqueue_refs::<VM>();
    }
}
impl<VM: VMBinding> RefEnqueue<VM> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
