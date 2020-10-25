use crate::plan::Plan;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

#[allow(dead_code)]
pub struct SanityChecker {
    refs: HashSet<ObjectReference>,
}

impl Default for SanityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl SanityChecker {
    pub fn new() -> Self {
        Self {
            refs: HashSet::new(),
        }
    }
}

#[derive(Default)]
pub struct ScheduleSanityGC;

impl<VM: VMBinding> GCWork<VM> for ScheduleSanityGC {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        worker.scheduler().reset_state();
        mmtk.plan.schedule_sanity_collection(worker.scheduler());
    }
}

pub struct SanityPrepare<P: Plan> {
    pub plan: &'static P,
}

unsafe impl<P: Plan> Sync for SanityPrepare<P> {}

impl<P: Plan> SanityPrepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for SanityPrepare<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        mmtk.plan.enter_sanity();
        {
            let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();
            sanity_checker.refs.clear();
        }
        let _guard = MUTATOR_ITERATOR_LOCK.lock().unwrap();
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::Mutator) };
            mmtk.scheduler
                .prepare_stage
                .add(PrepareMutator::<P>::new(&self.plan, mutator));
        }
        for w in &worker.group().unwrap().workers {
            w.local_works.add(PrepareCollector::default());
        }
    }
}

pub struct SanityRelease<P: Plan> {
    pub plan: &'static P,
}

unsafe impl<P: Plan> Sync for SanityRelease<P> {}

impl<P: Plan> SanityRelease<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for SanityRelease<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        mmtk.plan.leave_sanity();
        let _guard = MUTATOR_ITERATOR_LOCK.lock().unwrap();
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::Mutator) };
            mmtk.scheduler
                .release_stage
                .add(ReleaseMutator::<P>::new(self.plan, mutator));
        }
        for w in &worker.group().unwrap().workers {
            w.local_works.add(ReleaseCollector::default());
        }
    }
}

#[derive(Default)]
pub struct SanityGCProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<SanityGCProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> Deref for SanityGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for SanityGCProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl<VM: VMBinding> ProcessEdgesWork for SanityGCProcessEdges<VM> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges),
            ..Default::default()
        }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let mut sanity_checker = self.mmtk().sanity_checker.lock().unwrap();
        if !sanity_checker.refs.contains(&object) {
            // FIXME steveb consider VM-specific integrity check on reference.
            if !object.is_sane() {
                panic!("Invalid reference {:?}", object);
            }
            // Object is not "marked"
            sanity_checker.refs.insert(object); // "Mark" it
            ProcessEdgesWork::process_node(self, object);
        }
        object
    }
}
