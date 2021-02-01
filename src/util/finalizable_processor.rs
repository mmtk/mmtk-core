use crate::util::ObjectReference;
use crate::plan::TransitiveClosure;
use crate::plan::SelectedPlan;
use crate::vm::VMBinding;
use crate::scheduler::gc_works::ProcessEdgesWork;
use crate::scheduler::{GCWork, GCWorker};
use crate::MMTK;
use crate::plan::Plan;
use std::marker::PhantomData;

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

        for reff in self.candidates.drain(start..).collect::<Vec<ObjectReference>>() {
            if reff.is_live() {
                self.candidates.push(FinalizableProcessor::get_forwarded_finalizable(e, reff));
                continue;
            }

            let retained = FinalizableProcessor::return_for_finalize(e, reff);
            self.ready_for_finalize.push(retained);
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
        trace!("Finalization");
        let mut finalizable_processor = mmtk.finalizable_processor.lock().unwrap();
        let mut w = E::new(vec![], false);
        finalizable_processor.scan(&mut w, mmtk.plan.in_nursery());
        trace!("Finished finalization");
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
