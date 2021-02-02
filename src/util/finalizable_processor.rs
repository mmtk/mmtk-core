use crate::util::ObjectReference;
use crate::plan::TransitiveClosure;
use crate::plan::SelectedPlan;
use crate::vm::VMBinding;
use crate::scheduler::gc_works::ProcessEdgesWork;
use crate::scheduler::gc_works::ProcessEdgesBase;
use crate::scheduler::{GCWork, GCWorker};
use crate::MMTK;
use crate::plan::Plan;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct FinalizableProcessor {
    candidates: Vec<ObjectReference>,
    nursery_index: usize,
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

    fn get_forwarded_finalizable<E: ProcessEdgesWork>(e: &mut E, object: ObjectReference) -> ObjectReference {
        e.trace_object(object)
    }

    fn return_for_finalize<E: ProcessEdgesWork>(e: &mut E, object: ObjectReference) -> ObjectReference {
        e.trace_object(object)
    }

    pub fn scan<E: ProcessEdgesWork>(&mut self, e: &mut E, nursery: bool) {
        let start = if nursery {
            self.nursery_index
        } else {
            0
        };

        // We should go through ready_for_finalize objects and keep them alive.
        // Unlike candidates, those objects are known to be alive. This means 
        // theoratically we could do the following loop at any time in a GC (not necessarily after closure phase).
        // But we have to iterate through candidates after closure.
        self.candidates.append(&mut self.ready_for_finalize);

        for reff in self.candidates.drain(start..).collect::<Vec<ObjectReference>>() {
            trace!("Pop {:?} for finalization", reff);
            if reff.is_live() {
                let res = FinalizableProcessor::get_forwarded_finalizable(e, reff);
                trace!("{:?} is live, push {:?} back to candidates", reff, res);
                self.candidates.push(res);
                continue;
            }

            let retained = FinalizableProcessor::return_for_finalize(e, reff);
            self.ready_for_finalize.push(retained);
            trace!("{:?} is not live, push {:?} to ready_for_finalize", reff, retained);
        }

        self.nursery_index = self.candidates.len();
    }

    pub fn forward<E: ProcessEdgesWork>(&mut self, e: &mut E, _nursery: bool) {
        self.candidates.iter_mut().for_each(|reff| *reff = FinalizableProcessor::get_forwarded_finalizable(e, *reff));
    }

    pub fn get_ready_object(&mut self) -> Option<ObjectReference> {
        self.ready_for_finalize.pop()
    }
}

pub struct Finalization<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> GCWork<E::VM> for Finalization<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        debug!("Finalization, {} objects in candidates, {} objects ready to finalize", finalizable_processor.candidates.len(), finalizable_processor.ready_for_finalize.len());

        let mut w = E::new(vec![], false);
        w.mmtk = Some(mmtk);
        w.set_worker(worker);
        finalizable_processor.scan(&mut w, mmtk.plan.in_nursery());
        debug!("Finished finalization, {} objects in candidates, {} objects ready to finalize", finalizable_processor.candidates.len(), finalizable_processor.ready_for_finalize.len());
    }
}
impl<E: ProcessEdgesWork> Finalization<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct ForwardFinalization<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> GCWork<E::VM> for ForwardFinalization<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("Forward finalization");
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        let mut w = E::new(vec![], false);
        finalizable_processor.forward(&mut w, mmtk.plan.in_nursery());
        trace!("Finished forwarding finlizable");
    }
}
impl<E: ProcessEdgesWork> ForwardFinalization<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
