use super::*;
use crate::plan::global::GcStatus;
use crate::util::*;
use crate::vm::*;
use crate::*;
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct ScheduleCollection;

unsafe impl Sync for ScheduleCollection {}

impl<VM: VMBinding> GCWork<VM> for ScheduleCollection {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        mmtk.plan.schedule_collection(worker.scheduler());
    }
}

impl<VM: VMBinding> CoordinatorWork<MMTK<VM>> for ScheduleCollection {}

/// GC Preparation Work (include updating global states)
pub struct Prepare<P: Plan> {
    pub plan: &'static P,
}

unsafe impl<P: Plan> Sync for Prepare<P> {}

impl<P: Plan> Prepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for Prepare<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        trace!("Prepare Global");
        self.plan.prepare(worker.tls);
        let _guard = MUTATOR_ITERATOR_LOCK.lock().unwrap();
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            let mutator = unsafe { &mut *(mutator as *mut _ as *mut P::Mutator) };
            mmtk.scheduler
                .prepare_stage
                .add(PrepareMutator::<P>::new(self.plan, mutator));
        }
        for w in &worker.group().unwrap().workers {
            w.local_works.add(PrepareCollector::default());
        }
    }
}

/// GC Preparation Work (include updating global states)
pub struct PrepareMutator<P: Plan> {
    pub plan: &'static P,
    pub mutator: &'static mut P::Mutator,
}

unsafe impl<P: Plan> Sync for PrepareMutator<P> {}

impl<P: Plan> PrepareMutator<P> {
    pub fn new(plan: &'static P, mutator: &'static mut P::Mutator) -> Self {
        Self { plan, mutator }
    }
}

impl<P: Plan> GCWork<P::VM> for PrepareMutator<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, _mmtk: &'static MMTK<P::VM>) {
        trace!("Prepare Mutator");
        self.mutator.prepare(worker.tls);
    }
}

#[derive(Default)]
pub struct PrepareCollector;

impl<VM: VMBinding> GCWork<VM> for PrepareCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Prepare Collector");
        worker.local().prepare();
    }
}

pub struct Release<P: Plan> {
    pub plan: &'static P,
}

unsafe impl<P: Plan> Sync for Release<P> {}

impl<P: Plan> Release<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for Release<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        trace!("Release Global");
        self.plan.release(worker.tls);
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

pub struct ReleaseMutator<P: Plan> {
    pub plan: &'static P,
    pub mutator: &'static mut P::Mutator,
}

unsafe impl<P: Plan> Sync for ReleaseMutator<P> {}

impl<P: Plan> ReleaseMutator<P> {
    pub fn new(plan: &'static P, mutator: &'static mut P::Mutator) -> Self {
        Self { plan, mutator }
    }
}

impl<P: Plan> GCWork<P::VM> for ReleaseMutator<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, _mmtk: &'static MMTK<P::VM>) {
        trace!("Release Mutator");
        self.mutator.release(worker.tls);
    }
}

#[derive(Default)]
pub struct ReleaseCollector;

impl<VM: VMBinding> GCWork<VM> for ReleaseCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Collector");
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

impl<ScanEdges: ProcessEdgesWork> StopMutators<ScanEdges> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

lazy_static! {
    pub static ref MUTATOR_ITERATOR_LOCK: Mutex<()> = Mutex::new(());
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for StopMutators<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        if worker.is_coordinator() {
            trace!("stop_all_mutators start");
            debug_assert_eq!(SCANNED_STACKS.load(Ordering::SeqCst), 0);
            <E::VM as VMBinding>::VMCollection::stop_all_mutators::<E>(worker.tls);
            trace!("stop_all_mutators end");
            mmtk.scheduler.notify_mutators_paused(mmtk);
            if <E::VM as VMBinding>::VMScanning::SCAN_MUTATORS_IN_SAFEPOINT {
                let _guard = MUTATOR_ITERATOR_LOCK.lock().unwrap();
                // Prepare mutators if necessary
                // FIXME: This test is probably redundant. JikesRVM requires to call `prepare_mutator` once after mutators are paused
                if !mmtk.plan.common().stacks_prepared() {
                    for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
                        <E::VM as VMBinding>::VMCollection::prepare_mutator(
                            mutator.get_tls(),
                            mutator,
                        );
                    }
                }
                // Scan mutators
                if <E::VM as VMBinding>::VMScanning::SINGLE_THREAD_MUTATOR_SCANNING {
                    mmtk.scheduler.prepare_stage.add(ScanStackRoots::<E>::new());
                } else {
                    #[cfg(debug_assertions)]
                    let mut i = 0;
                    for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
                        #[cfg(debug_assertions)]
                        {
                            i += 1;
                        }
                        mmtk.scheduler
                            .prepare_stage
                            .add(ScanStackRoot::<E>(mutator));
                    }
                    #[cfg(debug_assertions)]
                    {
                        assert_eq!(<E::VM as VMBinding>::VMActivePlan::number_of_mutators(), i);
                    }
                }
            }
            mmtk.scheduler
                .prepare_stage
                .add(ScanVMSpecificRoots::<E>::new());
        } else {
            mmtk.scheduler
                .add_coordinator_work(StopMutators::<E>::new(), worker);
        }
    }
}

impl<E: ProcessEdgesWork> CoordinatorWork<MMTK<E::VM>> for StopMutators<E> {}

#[derive(Default)]
pub struct EndOfGC;

impl<VM: VMBinding> GCWork<VM> for EndOfGC {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        mmtk.plan.common().base.set_gc_status(GcStatus::NotInGC);
        <VM as VMBinding>::VMCollection::resume_mutators(worker.tls);
    }
}

impl<VM: VMBinding> CoordinatorWork<MMTK<VM>> for EndOfGC {}

static SCANNED_STACKS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct ScanStackRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl<E: ProcessEdgesWork> ScanStackRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanStackRoots<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanStackRoots");
        <E::VM as VMBinding>::VMScanning::scan_thread_roots::<E>();
        <E::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(false, worker.tls);
        mmtk.plan.common().base.set_gc_status(GcStatus::GcProper);
    }
}

pub struct ScanStackRoot<Edges: ProcessEdgesWork>(
    pub &'static mut Mutator<SelectedPlan<Edges::VM>>,
);

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanStackRoot<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanStackRoot for mutator {:?}", self.0.get_tls());
        <E::VM as VMBinding>::VMScanning::scan_thread_root::<E>(unsafe {
            &mut *(self.0 as *mut _)
        });
        self.0.flush();
        let old = SCANNED_STACKS.fetch_add(1, Ordering::SeqCst);
        if old + 1 == <E::VM as VMBinding>::VMActivePlan::number_of_mutators() {
            SCANNED_STACKS.store(0, Ordering::SeqCst);
            <E::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(
                false, worker.tls,
            );
            mmtk.plan.common().base.set_gc_status(GcStatus::GcProper);
        }
    }
}

#[derive(Default)]
pub struct ScanVMSpecificRoots<Edges: ProcessEdgesWork>(PhantomData<Edges>);

impl<E: ProcessEdgesWork> ScanVMSpecificRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanVMSpecificRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ScanStaticRoots");
        <E::VM as VMBinding>::VMScanning::scan_vm_specific_roots::<E>();
    }
}

#[derive(Default)]
pub struct ProcessEdgesBase<E: ProcessEdgesWork> {
    pub edges: Vec<Address>,
    pub nodes: Vec<ObjectReference>,
    pub mmtk: Option<&'static MMTK<E::VM>>,
    pub worker_tls: Option<OpaquePointer>,
}

impl<E: ProcessEdgesWork> ProcessEdgesBase<E> {
    pub fn new(edges: Vec<Address>) -> Self {
        Self {
            edges,
            nodes: vec![],
            mmtk: None,
            worker_tls: None,
        }
    }
    pub fn worker(&self) -> &'static mut GCWorker<E::VM> {
        <E::VM as VMBinding>::VMActivePlan::worker(self.worker_tls.unwrap())
    }
    pub fn mmtk(&self) -> &'static MMTK<E::VM> {
        self.mmtk.unwrap()
    }
    pub fn plan(&self) -> &'static SelectedPlan<E::VM> {
        &self.mmtk.unwrap().plan
    }
}

/// Scan & update a list of object slots
pub trait ProcessEdgesWork:
    Send + Sync + 'static + Sized + DerefMut + Deref<Target = ProcessEdgesBase<Self>>
{
    type VM: VMBinding;
    const CAPACITY: usize = 4096;
    const OVERWRITE_REFERENCE: bool = true;
    fn new(edges: Vec<Address>, roots: bool) -> Self;
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;

    #[inline]
    fn process_node(&mut self, object: ObjectReference) {
        if self.nodes.is_empty() {
            self.nodes.reserve(Self::CAPACITY);
        }
        self.nodes.push(object);
        if self.nodes.len() >= Self::CAPACITY {
            let mut new_nodes = Vec::with_capacity(Self::CAPACITY);
            mem::swap(&mut new_nodes, &mut self.nodes);
            self.mmtk
                .unwrap()
                .scheduler
                .closure_stage
                .add(ScanObjects::<Self>::new(new_nodes, false));
        }
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }

    #[inline]
    fn process_edges(&mut self) {
        for i in 0..self.edges.len() {
            self.process_edge(self.edges[i])
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for E {
    #[inline]
    default fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ProcessEdgesWork");
        self.mmtk = Some(mmtk);
        self.worker_tls = Some(worker.tls);
        self.process_edges();
        if !self.nodes.is_empty() {
            let mut new_nodes = Vec::with_capacity(Self::CAPACITY);
            mem::swap(&mut new_nodes, &mut self.nodes);
            self.mmtk
                .unwrap()
                .scheduler
                .closure_stage
                .add(ScanObjects::<Self>::new(new_nodes, false));
        }
        trace!("ProcessEdgesWork End");
    }
}

/// Scan & update a list of object slots
pub struct ScanObjects<Edges: ProcessEdgesWork> {
    buffer: Vec<ObjectReference>,
    #[allow(unused)]
    concurrent: bool,
    phantom: PhantomData<Edges>,
}

impl<Edges: ProcessEdgesWork> ScanObjects<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool) -> Self {
        Self {
            buffer,
            concurrent,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanObjects<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ScanObjects");
        <E::VM as VMBinding>::VMScanning::scan_objects::<E>(&self.buffer);
        trace!("ScanObjects End");
    }
}

#[derive(Default)]
pub struct ProcessModBuf<E: ProcessEdgesWork> {
    modified_nodes: Vec<ObjectReference>,
    modified_edges: Vec<Address>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessModBuf<E> {
    pub fn new(modified_nodes: Vec<ObjectReference>, modified_edges: Vec<Address>) -> Self {
        Self {
            modified_nodes,
            modified_edges,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessModBuf<E> {
    #[inline]
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        if mmtk.plan.in_nursery() {
            let mut modified_nodes = vec![];
            ::std::mem::swap(&mut modified_nodes, &mut self.modified_nodes);
            worker
                .scheduler()
                .closure_stage
                .add(ScanObjects::<E>::new(modified_nodes, false));

            let mut modified_edges = vec![];
            ::std::mem::swap(&mut modified_edges, &mut self.modified_edges);
            worker
                .scheduler()
                .closure_stage
                .add(E::new(modified_edges, true));
        } else {
            // Do nothing
        }
    }
}
