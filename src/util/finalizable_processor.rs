use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::{GCWork, GCWorker};
use crate::util::ObjectReference;
use crate::MMTK;
use std::marker::PhantomData;

/// A special processor for Finalizable objects.
// TODO: Should we consider if we want to merge FinalizableProcessor with ReferenceProcessor,
// and treat final reference as a special reference type in ReferenceProcessor.
#[derive(Default)]
pub struct FinalizableProcessor {
    /// Candidate objects that has finalizers with them
    candidates: Vec<ObjectReference>,
    /// Index into candidates to record where we are up to in the last scan of the candidates.
    /// Index after nursery_index are new objects inserted after the last GC.
    nursery_index: usize,
    /// Objects that can be finalized. They are actually dead, but we keep them alive
    /// until the binding pops them from the queue.
    ready_for_finalize: Vec<ObjectReference>,
}

impl FinalizableProcessor {
    pub fn new() -> Self {
        Self {
            candidates: vec![],
            nursery_index: 0,
            ready_for_finalize: vec![],
        }
    }

    pub fn add(&mut self, object: ObjectReference) {
        self.candidates.push(object);
    }

    fn get_forwarded_finalizable<E: ProcessEdgesWork>(
        e: &mut E,
        object: ObjectReference,
    ) -> ObjectReference {
        e.trace_object(object)
    }

    fn return_for_finalize<E: ProcessEdgesWork>(
        e: &mut E,
        object: ObjectReference,
    ) -> ObjectReference {
        e.trace_object(object)
    }

    pub fn scan<E: ProcessEdgesWork>(&mut self, e: &mut E, nursery: bool) {
        let start = if nursery { self.nursery_index } else { 0 };

        // We should go through ready_for_finalize objects and keep them alive.
        // Unlike candidates, those objects are known to be alive. This means
        // theoratically we could do the following loop at any time in a GC (not necessarily after closure phase).
        // But we have to iterate through candidates after closure.
        self.candidates.append(&mut self.ready_for_finalize);

        for reff in self
            .candidates
            .drain(start..)
            .collect::<Vec<ObjectReference>>()
        {
            trace!("Pop {:?} for finalization", reff);
            if reff.is_live() {
                let res = FinalizableProcessor::get_forwarded_finalizable(e, reff);
                trace!("{:?} is live, push {:?} back to candidates", reff, res);
                self.candidates.push(res);
                continue;
            }

            let retained = FinalizableProcessor::return_for_finalize(e, reff);
            self.ready_for_finalize.push(retained);
            trace!(
                "{:?} is not live, push {:?} to ready_for_finalize",
                reff,
                retained
            );
        }
        e.flush();

        self.nursery_index = self.candidates.len();
    }

    pub fn forward<E: ProcessEdgesWork>(&mut self, e: &mut E, _nursery: bool) {
        self.candidates
            .iter_mut()
            .for_each(|reff| *reff = FinalizableProcessor::get_forwarded_finalizable(e, *reff));
        e.flush();
    }

    pub fn get_ready_object(&mut self) -> Option<ObjectReference> {
        self.ready_for_finalize.pop()
    }
}

#[derive(Default)]
pub struct Finalization<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> GCWork<E::VM> for Finalization<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        debug!(
            "Finalization, {} objects in candidates, {} objects ready to finalize",
            finalizable_processor.candidates.len(),
            finalizable_processor.ready_for_finalize.len()
        );

        let mut w = E::new(vec![], false, mmtk);
        w.set_worker(worker);
        finalizable_processor.scan(&mut w, mmtk.plan.in_nursery());
        debug!(
            "Finished finalization, {} objects in candidates, {} objects ready to finalize",
            finalizable_processor.candidates.len(),
            finalizable_processor.ready_for_finalize.len()
        );
    }
}
impl<E: ProcessEdgesWork> Finalization<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct ForwardFinalization<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> GCWork<E::VM> for ForwardFinalization<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("Forward finalization");
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        let mut w = E::new(vec![], false, mmtk);
        finalizable_processor.forward(&mut w, mmtk.plan.in_nursery());
        trace!("Finished forwarding finlizable");
    }
}
impl<E: ProcessEdgesWork> ForwardFinalization<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
