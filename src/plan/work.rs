use super::worker::*;
use super::scheduler::*;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use crate::plan::Plan;
use std::sync::{Arc, Barrier};
use std::marker::PhantomData;
use crate::vm::*;
use crate::util::{ObjectReference, Address, OpaquePointer};
use crate::plan::{TransitiveClosure, SelectedPlan, MutatorContext};
use std::mem;
use std::ops::{Deref, DerefMut};


pub trait GenericWork<VM: VMBinding>: 'static + Send + Sync {
    fn requires_stop_the_world(&self) -> bool { false }
    fn do_work(&mut self, worker: &Worker<VM>, mmtk: &'static MMTK<VM>);
}

impl <VM: VMBinding, W: Work<VM=VM>> GenericWork<VM> for W {
    fn requires_stop_the_world(&self) -> bool {
        W::REQUIRES_STOP_THE_WORLD
    }
    fn do_work(&mut self, worker: &Worker<VM>, mmtk: &'static MMTK<VM>) {
        Work::do_work(self, worker, mmtk)
    }
}

impl <VM: VMBinding> PartialEq for Box<dyn GenericWork<VM>> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() as *const dyn GenericWork<VM> == other.as_ref() as *const dyn GenericWork<VM>
    }
}

impl <VM: VMBinding> Eq for Box<dyn GenericWork<VM>> {}

pub trait Work: 'static + Send + Sync + Sized {
    type VM: VMBinding;
    const REQUIRES_STOP_THE_WORLD: bool = false;
    fn do_work(&mut self, worker: &Worker<Self::VM>, mmtk: &'static MMTK<Self::VM>);
}



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

impl <P: Plan> Work for Prepare<P> {
    type VM = P::VM;
    const REQUIRES_STOP_THE_WORLD: bool = true;
    fn do_work(&mut self, worker: &Worker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Prepare Global");
        self.plan.prepare(worker.tls);
        <Self::VM as VMBinding>::VMActivePlan::reset_mutator_iterator();
        while let Some(mutator) = <P::VM as VMBinding>::VMActivePlan::get_next_mutator() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::MutatorT) };
            println!("Scuedule Prepare Mutator");
            mmtk.scheduler.add_with_highest_priority(PrepareMutator::<P>::new(self.plan, mutator));
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

impl <P: Plan> Work for PrepareMutator<P> {
    type VM = P::VM;
    const REQUIRES_STOP_THE_WORLD: bool = true;
    fn do_work(&mut self, worker: &Worker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        println!("Prepare Mutator");
        self.mutator.prepare(worker.tls);
    }
}

/// Stop all mutators
///
/// Schedule a `ScanStackRoots` immediately after a mutator is paused
///
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<ScanMutators: ScanMutatorsWork>(PhantomData<ScanMutators>);

impl <ScanMutators: ScanMutatorsWork> StopMutators<ScanMutators> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl <ScanMutators: ScanMutatorsWork> Work for StopMutators<ScanMutators> {
    type VM = <ScanMutators as Work>::VM;
    fn do_work(&mut self, worker: &Worker<Self::VM>, mmtk: &'static MMTK<Self::VM>) {
        println!("stop_all_mutators start");
        <Self::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls);
        println!("stop_all_mutators end");
        mmtk.scheduler.mutators_stopped();
        mmtk.scheduler.add_with_highest_priority(ScanMutators::new());
    }
}

pub trait ScanMutatorsWork: Work {
    fn new() -> Self;
}

#[derive(Default)]
pub struct ScanStackRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl <E: ProcessEdgesWork> Work for ScanStackRoots<E> {
    type VM = E::VM;
    fn do_work(&mut self, worker: &Worker<Self::VM>, mmtk: &'static MMTK<Self::VM>) {
        <E::VM as VMBinding>::VMScanning::scan_thread_roots::<E>(worker.tls);
    }
}

impl <E: ProcessEdgesWork> ScanMutatorsWork for ScanStackRoots<E> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct ProcessEdgesBase<E: ProcessEdgesWork> {
    pub edges: Vec<Address>,
    pub nodes: Vec<ObjectReference>,
    pub mmtk: Option<&'static MMTK<E::VM>>,
    pub tls: OpaquePointer,
}

impl <E: ProcessEdgesWork> ProcessEdgesBase<E> {
    pub fn new(edges: Vec<Address>) -> Self {
        Self { edges, nodes: vec![], mmtk: None, tls: OpaquePointer::UNINITIALIZED }
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
            self.mmtk.unwrap().scheduler.add_with_highest_priority(ScanObjects::<Self>::new(new_nodes, false));
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

impl <E: ProcessEdgesWork> Work for E {
    type VM = <E as ProcessEdgesWork>::VM;
    const REQUIRES_STOP_THE_WORLD: bool = true;
    default fn do_work(&mut self, worker: &Worker<Self::VM>, mmtk: &'static MMTK<Self::VM>) {
        println!("ProcessEdgesWork");
        self.mmtk = Some(mmtk);
        self.tls = worker.tls;
        self.process_edges();
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

impl <Edges: ProcessEdgesWork> Work for ScanObjects<Edges> {
    type VM = <Edges as Work>::VM;
    const REQUIRES_STOP_THE_WORLD: bool = true;
    fn do_work(&mut self, worker: &Worker<Self::VM>, mmtk: &'static MMTK<Self::VM>) {
        println!("ScanObjects");
        <Self::VM as VMBinding>::VMScanning::scan_objects::<Edges>(&self.buffer);
    }
}
