use super::*;
use crate::*;
use crate::vm::*;
use crate::util::*;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::mem;



/// GC Preparation Work (include updating global states)
pub struct Prepare<P: Plan> {
    pub plan: &'static P,
}

unsafe impl <P: Plan> Sync for Prepare<P> {}

impl <P: Plan> Prepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl <P: Plan> GCWork<P::VM> for Prepare<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Prepare Global");
        self.plan.prepare(worker.tls);
        <P::VM as VMBinding>::VMActivePlan::reset_mutator_iterator();
        while let Some(mutator) = <P::VM as VMBinding>::VMActivePlan::get_next_mutator() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::MutatorT) };
            mmtk.scheduler.prepare_stage.add(PrepareMutator::<P>::new(self.plan, mutator));
        }
        for w in &worker.group().workers {
            w.local_works.add(PrepareCollector::default());
        }
    }
}

/// GC Preparation Work (include updating global states)
pub struct PrepareMutator<P: Plan> {
    pub plan: &'static P,
    pub mutator: &'static mut P::MutatorT,
}

unsafe impl <P: Plan> Sync for PrepareMutator<P> {}

impl <P: Plan> PrepareMutator<P> {
    pub fn new(plan: &'static P, mutator: &'static mut P::MutatorT) -> Self {
        Self { plan, mutator }
    }
}

impl <P: Plan> GCWork<P::VM> for PrepareMutator<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Prepare Mutator");
        self.mutator.prepare(worker.tls);
    }
}

#[derive(Default)]
pub struct PrepareCollector;

impl <VM: VMBinding> GCWork<VM> for PrepareCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        println!("Prepare Collector");
        worker.local().prepare();
    }
}

pub struct Release<P: Plan> {
    pub plan: &'static P,
}

unsafe impl <P: Plan> Sync for Release<P> {}

impl <P: Plan> Release<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl <P: Plan> GCWork<P::VM> for Release<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Release Global");
        self.plan.release(worker.tls);
        <P::VM as VMBinding>::VMActivePlan::reset_mutator_iterator();
        while let Some(mutator) = <P::VM as VMBinding>::VMActivePlan::get_next_mutator() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::MutatorT) };
            mmtk.scheduler.release_stage.add(ReleaseMutator::<P>::new(self.plan, mutator));
        }
        for w in &worker.group().workers {
            w.local_works.add(ReleaseCollector::default());
        }
    }
}

pub struct ReleaseMutator<P: Plan> {
    pub plan: &'static P,
    pub mutator: &'static mut P::MutatorT,
}

unsafe impl <P: Plan> Sync for ReleaseMutator<P> {}

impl <P: Plan> ReleaseMutator<P> {
    pub fn new(plan: &'static P, mutator: &'static mut P::MutatorT) -> Self {
        Self { plan, mutator }
    }
}

impl <P: Plan> GCWork<P::VM> for ReleaseMutator<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Release Mutator");
        self.mutator.release(worker.tls);
    }
}

#[derive(Default)]
pub struct ReleaseCollector;

impl <VM: VMBinding> GCWork<VM> for ReleaseCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        println!("Release Collector");
        worker.local().release();
    }
}

/// Stop all mutators
///
/// Schedule a `ScanStackRoots` immediately after a mutator is paused
///
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<ScanEdges: ProcessEdgesWork>(PhantomData<ScanEdges>);

impl <ScanEdges: ProcessEdgesWork> StopMutators<ScanEdges> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for StopMutators<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("stop_all_mutators start");
        <E::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls);
        println!("stop_all_mutators end");
        mmtk.scheduler.notify_mutators_paused(mmtk);
        mmtk.scheduler.prepare_stage.add(ScanStackRoots::<E>::new());
        mmtk.scheduler.prepare_stage.add_with_priority(0, ScanStaticRoots::<E>::new());
        mmtk.scheduler.prepare_stage.add_with_priority(0, ScanGlobalRoots::<E>::new());
    }
}

#[derive(Default)]
pub struct ResumeMutators;

impl <VM: VMBinding> GCWork<VM> for ResumeMutators {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        println!("ResumeMutators");
        <VM as VMBinding>::VMCollection::resume_mutators(worker.tls);
    }
}

#[derive(Default)]
pub struct ScanStackRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl <E: ProcessEdgesWork> ScanStackRoots<E> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for ScanStackRoots<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("ScanStackRoots");
        <E::VM as VMBinding>::VMScanning::scan_thread_roots::<E>();
    }
}

#[derive(Default)]
pub struct ScanStaticRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl <E: ProcessEdgesWork> ScanStaticRoots<E> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for ScanStaticRoots<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("ScanStaticRoots");
        <E::VM as VMBinding>::VMScanning::scan_static_roots::<E>();
    }
}

#[derive(Default)]
pub struct ScanGlobalRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl <E: ProcessEdgesWork> ScanGlobalRoots<E> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for ScanGlobalRoots<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("ScanGlobalRoots");
        <E::VM as VMBinding>::VMScanning::scan_global_roots::<E>();
    }
}

#[derive(Default)]
pub struct ProcessEdgesBase<E: ProcessEdgesWork> {
    pub edges: Vec<Address>,
    pub nodes: Vec<ObjectReference>,
    pub mmtk: Option<&'static MMTK<E::VM>>,
    pub worker: Option<&'static GCWorker<E::VM>>,
}

impl <E: ProcessEdgesWork> ProcessEdgesBase<E> {
    pub fn new(edges: Vec<Address>) -> Self {
        Self { edges, nodes: vec![], mmtk: None, worker: None }
    }
    pub fn worker(&self) -> &'static GCWorker<E::VM> {
        &self.worker.unwrap()
    }
    pub fn mmtk(&self) -> &'static MMTK<E::VM> {
        self.mmtk.unwrap()
    }
    pub fn plan(&self) -> &'static SelectedPlan<E::VM> {
        &self.mmtk.unwrap().plan
    }
}

/// Scan & update a list of object slots
pub trait ProcessEdgesWork: Send + Sync + 'static + Sized + DerefMut + Deref<Target=ProcessEdgesBase<Self>> {
    type VM: VMBinding;
    const CAPACITY: usize = 512;
    const OVERWRITE_REFERENCE: bool = true;
    fn new(edges: Vec<Address>, roots: bool) -> Self;
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;

    fn process_node(&mut self, object: ObjectReference) {
        if self.nodes.len() == 0 {
            self.nodes.reserve(Self::CAPACITY);
        }
        self.nodes.push(object);
        if self.nodes.len() >= Self::CAPACITY {
            let mut new_nodes = Vec::with_capacity(Self::CAPACITY);
            mem::swap(&mut new_nodes, &mut self.nodes);
            self.mmtk.unwrap().scheduler.closure_stage.add(ScanObjects::<Self>::new(new_nodes, false));
        }
    }

    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }

    fn process_edges(&mut self) {
        for i in 0..self.edges.len() {
            self.process_edge(self.edges[i])
        }
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for E {
    default fn do_work(&mut self, worker: &'static mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("ProcessEdgesWork");
        self.mmtk = Some(mmtk);
        self.worker = Some(worker);
        self.process_edges();
        if self.nodes.len() > 0 {
            let mut new_nodes = Vec::with_capacity(Self::CAPACITY);
            mem::swap(&mut new_nodes, &mut self.nodes);
            self.mmtk.unwrap().scheduler.closure_stage.add(ScanObjects::<Self>::new(new_nodes, false));
        }
        println!("ProcessEdgesWork End");
    }
}

/// Scan & update a list of object slots
pub struct ScanObjects<Edges: ProcessEdgesWork> {
    buffer: Vec<ObjectReference>,
    concurrent: bool,
    phantom: PhantomData<Edges>,
}

impl <Edges: ProcessEdgesWork> ScanObjects<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool) -> Self {
        Self { buffer, concurrent, phantom: PhantomData }
    }
}

impl <E: ProcessEdgesWork> GCWork<E::VM> for ScanObjects<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        println!("ScanObjects");
        <E::VM as VMBinding>::VMScanning::scan_objects::<E>(&self.buffer);
        println!("ScanObjects End");
    }
}
