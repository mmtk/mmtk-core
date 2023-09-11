use crate::plan::is_nursery_gc;
use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::{GCWork, GCWorker, WorkBucketStage};
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::Finalizable;
use crate::vm::{Collection, VMBinding};
use crate::MMTK;
use std::marker::PhantomData;

/// A special processor for Finalizable objects.
// TODO: we should consider if we want to merge FinalizableProcessor with ReferenceProcessor,
// and treat final reference as a special reference type in ReferenceProcessor.
#[derive(Default)]
pub struct FinalizableProcessor<F: Finalizable> {
    /// Candidate objects that has finalizers with them
    candidates: Vec<F>,
    /// Index into candidates to record where we are up to in the last scan of the candidates.
    /// Index after nursery_index are new objects inserted after the last GC.
    nursery_index: usize,
    /// Objects that can be finalized. They are actually dead, but we keep them alive
    /// until the binding pops them from the queue.
    ready_for_finalize: Vec<F>,
}

impl<F: Finalizable> FinalizableProcessor<F> {
    pub fn new() -> Self {
        Self {
            candidates: vec![],
            nursery_index: 0,
            ready_for_finalize: vec![],
        }
    }

    pub fn add(&mut self, object: F) {
        self.candidates.push(object);
    }

    fn forward_finalizable_reference<E: ProcessEdgesWork>(e: &mut E, finalizable: &mut F) {
        finalizable.keep_alive::<E>(e);
    }

    pub fn scan<E: ProcessEdgesWork>(&mut self, tls: VMWorkerThread, e: &mut E, nursery: bool) {
        let start = if nursery { self.nursery_index } else { 0 };

        // We should go through ready_for_finalize objects and keep them alive.
        // Unlike candidates, those objects are known to be alive. This means
        // theoratically we could do the following loop at any time in a GC (not necessarily after closure phase).
        // But we have to iterate through candidates after closure.
        self.candidates.append(&mut self.ready_for_finalize);
        debug_assert!(self.ready_for_finalize.is_empty());

        for mut f in self.candidates.drain(start..).collect::<Vec<F>>() {
            let reff = f.get_reference();
            trace!("Pop {:?} for finalization", reff);
            if reff.is_live() {
                FinalizableProcessor::<F>::forward_finalizable_reference(e, &mut f);
                trace!("{:?} is live, push {:?} back to candidates", reff, f);
                self.candidates.push(f);
                continue;
            }

            // We should not at this point mark the object as live. A binding may register an object
            // multiple times with different finalizer methods. If we mark the object as live here, and encounter
            // the same object later in the candidates list (possibly with a different finalizer method),
            // we will erroneously think the object never died, and won't push it to the ready_to_finalize
            // queue.
            // So we simply push the object to the ready_for_finalize queue, and mark them as live objects later.
            self.ready_for_finalize.push(f);
        }

        // Keep the finalizable objects alive.
        self.forward_finalizable(e, nursery);

        // Set nursery_index to the end of the candidates (the candidates before the index are scanned)
        self.nursery_index = self.candidates.len();

        <<E as ProcessEdgesWork>::VM as VMBinding>::VMCollection::schedule_finalization(tls);
    }

    pub fn forward_candidate<E: ProcessEdgesWork>(&mut self, e: &mut E, _nursery: bool) {
        self.candidates
            .iter_mut()
            .for_each(|f| FinalizableProcessor::<F>::forward_finalizable_reference(e, f));
        e.flush();
    }

    pub fn forward_finalizable<E: ProcessEdgesWork>(&mut self, e: &mut E, _nursery: bool) {
        self.ready_for_finalize
            .iter_mut()
            .for_each(|f| FinalizableProcessor::<F>::forward_finalizable_reference(e, f));
        e.flush();
    }

    pub fn get_ready_object(&mut self) -> Option<F> {
        self.ready_for_finalize.pop()
    }

    pub fn get_all_finalizers(&mut self) -> Vec<F> {
        let mut ret = std::mem::take(&mut self.candidates);
        let ready_objects = std::mem::take(&mut self.ready_for_finalize);
        ret.extend(ready_objects);

        // We removed objects from candidates. Reset nursery_index
        self.nursery_index = 0;

        ret
    }

    pub fn get_finalizers_for(&mut self, object: ObjectReference) -> Vec<F> {
        // Drain filter for finalizers that equal to 'object':
        // * for elements that equal to 'object', they will be removed from the original vec, and returned.
        // * for elements that do not equal to 'object', they will be left in the original vec.
        // TODO: We should replace this with `vec.drain_filter()` when it is stablized.
        let drain_filter = |vec: &mut Vec<F>| -> Vec<F> {
            let mut i = 0;
            let mut ret = vec![];
            while i < vec.len() {
                if vec[i].get_reference() == object {
                    let val = vec.remove(i);
                    ret.push(val);
                } else {
                    i += 1;
                }
            }
            ret
        };
        let mut ret: Vec<F> = drain_filter(&mut self.candidates);
        ret.extend(drain_filter(&mut self.ready_for_finalize));

        // We removed objects from candidates. Reset nursery_index
        self.nursery_index = 0;

        ret
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

        let mut w = E::new(vec![], false, mmtk, WorkBucketStage::FinalRefClosure);
        w.set_worker(worker);
        finalizable_processor.scan(worker.tls, &mut w, is_nursery_gc(mmtk.get_plan()));
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
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("Forward finalization");
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        let mut w = E::new(vec![], false, mmtk, WorkBucketStage::FinalizableForwarding);
        w.set_worker(worker);
        finalizable_processor.forward_candidate(&mut w, is_nursery_gc(mmtk.get_plan()));

        finalizable_processor.forward_finalizable(&mut w, is_nursery_gc(mmtk.get_plan()));
        trace!("Finished forwarding finlizable");
    }
}
impl<E: ProcessEdgesWork> ForwardFinalization<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
