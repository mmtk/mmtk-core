use super::work_bucket::WorkBucketStage;
use super::*;
use crate::plan::GcStatus;
use crate::util::metadata::*;
use crate::util::*;
use crate::vm::*;
use crate::*;
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

pub struct ScheduleCollection;

impl<VM: VMBinding> GCWork<VM> for ScheduleCollection {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        mmtk.plan.schedule_collection(worker.scheduler());
    }
}

impl<VM: VMBinding> CoordinatorWork<VM> for ScheduleCollection {}

/// The global GC Preparation Work
/// This work packet invokes prepare() for the plan (which will invoke prepare() for each space), and
/// pushes work packets for preparing mutators and collectors.
/// We should only have one such work packet per GC, before any actual GC work starts.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Prepare<P: Plan, W: CopyContext + GCWorkerLocal> {
    pub plan: &'static P,
    _p: PhantomData<W>,
}

impl<P: Plan, W: CopyContext + GCWorkerLocal> Prepare<P, W> {
    pub fn new(plan: &'static P) -> Self {
        Self {
            plan,
            _p: PhantomData,
        }
    }
}

impl<P: Plan, W: CopyContext + GCWorkerLocal> GCWork<P::VM> for Prepare<P, W> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        trace!("Prepare Global");
        // We assume this is the only running work packet that accesses plan at the point of execution
        #[allow(clippy::cast_ref_to_mut)]
        let plan_mut: &mut P = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.prepare(worker.tls);

        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(PrepareMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group().workers {
            w.local_work_bucket.add(PrepareCollector::<W>::new());
        }
    }
}

/// The mutator GC Preparation Work
pub struct PrepareMutator<VM: VMBinding> {
    // The mutator reference has static lifetime.
    // It is safe because the actual lifetime of this work-packet will not exceed the lifetime of a GC.
    pub mutator: &'static mut Mutator<VM>,
}

impl<VM: VMBinding> PrepareMutator<VM> {
    pub fn new(mutator: &'static mut Mutator<VM>) -> Self {
        Self { mutator }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareMutator<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Prepare Mutator");
        self.mutator.prepare(worker.tls);
    }
}

/// The collector GC Preparation Work
#[derive(Default)]
pub struct PrepareCollector<W: CopyContext + GCWorkerLocal>(PhantomData<W>);

impl<W: CopyContext + GCWorkerLocal> PrepareCollector<W> {
    pub fn new() -> Self {
        PrepareCollector(PhantomData)
    }
}

impl<VM: VMBinding, W: CopyContext + GCWorkerLocal> GCWork<VM> for PrepareCollector<W> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Prepare Collector");
        unsafe { worker.local::<W>() }.prepare();
    }
}

/// The global GC release Work
/// This work packet invokes release() for the plan (which will invoke release() for each space), and
/// pushes work packets for releasing mutators and collectors.
/// We should only have one such work packet per GC, after all actual GC work ends.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Release<P: Plan, W: CopyContext + GCWorkerLocal> {
    pub plan: &'static P,
    _p: PhantomData<W>,
}

impl<P: Plan, W: CopyContext + GCWorkerLocal> Release<P, W> {
    pub fn new(plan: &'static P) -> Self {
        Self {
            plan,
            _p: PhantomData,
        }
    }
}

impl<P: Plan, W: CopyContext + GCWorkerLocal> GCWork<P::VM> for Release<P, W> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        trace!("Release Global");
        <P::VM as VMBinding>::VMCollection::vm_release();
        // We assume this is the only running work packet that accesses plan at the point of execution
        #[allow(clippy::cast_ref_to_mut)]
        let plan_mut: &mut P = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.release(worker.tls);

        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Release]
                .add(ReleaseMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group().workers {
            w.local_work_bucket.add(ReleaseCollector::<W>::new());
        }
        // TODO: Process weak references properly
        mmtk.reference_processors.clear();
    }
}

/// The mutator release Work
pub struct ReleaseMutator<VM: VMBinding> {
    // The mutator reference has static lifetime.
    // It is safe because the actual lifetime of this work-packet will not exceed the lifetime of a GC.
    pub mutator: &'static mut Mutator<VM>,
}

impl<VM: VMBinding> ReleaseMutator<VM> {
    pub fn new(mutator: &'static mut Mutator<VM>) -> Self {
        Self { mutator }
    }
}

impl<VM: VMBinding> GCWork<VM> for ReleaseMutator<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Mutator");
        self.mutator.release(worker.tls);
    }
}

/// The collector release Work
#[derive(Default)]
pub struct ReleaseCollector<W: CopyContext + GCWorkerLocal>(PhantomData<W>);

impl<W: CopyContext + GCWorkerLocal> ReleaseCollector<W> {
    pub fn new() -> Self {
        ReleaseCollector(PhantomData)
    }
}

impl<VM: VMBinding, W: CopyContext + GCWorkerLocal> GCWork<VM> for ReleaseCollector<W> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Collector");
        unsafe { worker.local::<W>() }.release();
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

impl<E: ProcessEdgesWork> GCWork<E::VM> for StopMutators<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        if worker.is_coordinator() {
            trace!("stop_all_mutators start");
            debug_assert_eq!(mmtk.plan.base().scanned_stacks.load(Ordering::SeqCst), 0);
            <E::VM as VMBinding>::VMCollection::stop_all_mutators::<E>(worker.tls);
            trace!("stop_all_mutators end");
            mmtk.scheduler.notify_mutators_paused(mmtk);
            if <E::VM as VMBinding>::VMScanning::SCAN_MUTATORS_IN_SAFEPOINT {
                // Prepare mutators if necessary
                // FIXME: This test is probably redundant. JikesRVM requires to call `prepare_mutator` once after mutators are paused
                if !mmtk.plan.base().stacks_prepared() {
                    for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
                        <E::VM as VMBinding>::VMCollection::prepare_mutator(
                            worker.tls,
                            mutator.get_tls(),
                            mutator,
                        );
                    }
                }
                // Scan mutators
                if <E::VM as VMBinding>::VMScanning::SINGLE_THREAD_MUTATOR_SCANNING {
                    mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                        .add(ScanStackRoots::<E>::new());
                } else {
                    for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
                        mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                            .add(ScanStackRoot::<E>(mutator));
                    }
                }
            }
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(ScanVMSpecificRoots::<E>::new());
        } else {
            mmtk.scheduler
                .add_coordinator_work(StopMutators::<E>::new(), worker);
        }
    }
}

impl<E: ProcessEdgesWork> CoordinatorWork<E::VM> for StopMutators<E> {}

#[derive(Default)]
pub struct EndOfGC;

impl<VM: VMBinding> GCWork<VM> for EndOfGC {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        info!("End of GC");

        #[cfg(feature = "extreme_assertions")]
        if crate::util::edge_logger::should_check_duplicate_edges(&*mmtk.plan) {
            // reset the logging info at the end of each GC
            crate::util::edge_logger::reset();
        }

        mmtk.plan.base().set_gc_status(GcStatus::NotInGC);
        <VM as VMBinding>::VMCollection::resume_mutators(worker.tls);
    }
}

impl<VM: VMBinding> CoordinatorWork<VM> for EndOfGC {}

/// Delegate to the VM binding for reference processing.
///
/// Some VMs (e.g. v8) do not have a Java-like global weak reference storage, and the
/// processing of those weakrefs may be more complex. For such case, we delegate to the
/// VM binding to process weak references.
#[derive(Default)]
pub struct ProcessWeakRefs<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> ProcessWeakRefs<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessWeakRefs<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ProcessWeakRefs");
        <E::VM as VMBinding>::VMCollection::process_weak_refs::<E>(worker);
    }
}

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
        for mutator in <E::VM as VMBinding>::VMActivePlan::mutators() {
            mutator.flush();
        }
        mmtk.plan.common().base.set_gc_status(GcStatus::GcProper);
    }
}

pub struct ScanStackRoot<Edges: ProcessEdgesWork>(pub &'static mut Mutator<Edges::VM>);

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanStackRoot<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanStackRoot for mutator {:?}", self.0.get_tls());
        let base = &mmtk.plan.base();
        let mutators = <E::VM as VMBinding>::VMActivePlan::number_of_mutators();
        <E::VM as VMBinding>::VMScanning::scan_thread_root::<E>(
            unsafe { &mut *(self.0 as *mut _) },
            worker.tls,
        );
        self.0.flush();
        let old = base.scanned_stacks.fetch_add(1, Ordering::SeqCst);
        trace!(
            "mutator {:?} old scanned_stacks = {}, new scanned_stacks = {}",
            self.0.get_tls(),
            old,
            base.scanned_stacks.load(Ordering::Relaxed)
        );

        if old + 1 >= mutators {
            loop {
                let current = base.scanned_stacks.load(Ordering::Relaxed);
                if current < mutators {
                    break;
                } else if base.scanned_stacks.compare_exchange(
                    current,
                    current - mutators,
                    Ordering::Release,
                    Ordering::Relaxed,
                ) == Ok(current)
                {
                    trace!(
                        "mutator {:?} old scanned_stacks = {}, new scanned_stacks = {}, number_of_mutators = {}",
                        self.0.get_tls(),
                        current,
                        base.scanned_stacks.load(Ordering::Relaxed),
                        mutators
                    );
                    <E::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(
                        false, worker.tls,
                    );
                    base.set_gc_status(GcStatus::GcProper);
                    break;
                }
            }
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

pub struct ProcessEdgesBase<E: ProcessEdgesWork> {
    pub edges: Vec<Address>,
    pub nodes: Vec<ObjectReference>,
    mmtk: &'static MMTK<E::VM>,
    // Use raw pointer for fast pointer dereferencing, instead of using `Option<&'static mut GCWorker<E::VM>>`.
    // Because a copying gc will dereference this pointer at least once for every object copy.
    worker: *mut GCWorker<E::VM>,
}

unsafe impl<E: ProcessEdgesWork> Send for ProcessEdgesBase<E> {}

impl<E: ProcessEdgesWork> ProcessEdgesBase<E> {
    // Requires an MMTk reference. Each plan-specific type that uses ProcessEdgesBase can get a static plan reference
    // at creation. This avoids overhead for dynamic dispatch or downcasting plan for each object traced.
    pub fn new(edges: Vec<Address>, mmtk: &'static MMTK<E::VM>) -> Self {
        #[cfg(feature = "extreme_assertions")]
        if crate::util::edge_logger::should_check_duplicate_edges(&*mmtk.plan) {
            for edge in &edges {
                // log edge, panic if already logged
                crate::util::edge_logger::log_edge(*edge);
            }
        }
        Self {
            edges,
            nodes: vec![],
            mmtk,
            worker: std::ptr::null_mut(),
        }
    }
    pub fn set_worker(&mut self, worker: &mut GCWorker<E::VM>) {
        self.worker = worker;
    }
    #[inline]
    pub fn worker(&self) -> &'static mut GCWorker<E::VM> {
        unsafe { &mut *self.worker }
    }
    #[inline]
    pub fn mmtk(&self) -> &'static MMTK<E::VM> {
        self.mmtk
    }
    #[inline]
    pub fn plan(&self) -> &'static dyn Plan<VM = E::VM> {
        &*self.mmtk.plan
    }
    /// Pop all nodes from nodes, and clear nodes to an empty vector.
    #[inline]
    pub fn pop_nodes(&mut self) -> Vec<ObjectReference> {
        debug_assert!(
            !self.nodes.is_empty(),
            "Attempted to flush nodes in ProcessEdgesWork while nodes set is empty."
        );
        let mut new_nodes = vec![];
        mem::swap(&mut new_nodes, &mut self.nodes);
        new_nodes
    }
}

/// Scan & update a list of object slots
pub trait ProcessEdgesWork:
    Send + 'static + Sized + DerefMut + Deref<Target = ProcessEdgesBase<Self>>
{
    type VM: VMBinding;
    const CAPACITY: usize = 4096;
    const OVERWRITE_REFERENCE: bool = true;
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;
    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<Self::VM>) -> Self;
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;

    #[inline]
    fn process_node(&mut self, object: ObjectReference) {
        if self.nodes.is_empty() {
            self.nodes.reserve(Self::CAPACITY);
        }
        self.nodes.push(object);
        // No need to flush this `nodes` local buffer to some global pool.
        // The max length of `nodes` buffer is equal to `CAPACITY` (when every edge produces a node)
        // So maximum 1 `ScanObjects` work can be created from `nodes` buffer
    }

    /// Create a new scan work packet. If SCAN_OBJECTS_IMMEDIATELY, the work packet will be executed immediately, in this method.
    /// Otherwise, the work packet will be added the Closure work bucket and will be dispatched later by the scheduler.
    #[inline]
    fn new_scan_work(&mut self, work_packet: impl GCWork<Self::VM>) {
        if Self::SCAN_OBJECTS_IMMEDIATELY {
            // We execute this `scan_objects_work` immediately.
            // This is expected to be a useful optimization because,
            // say for _pmd_ with 200M heap, we're likely to have 50000~60000 `ScanObjects` work packets
            // being dispatched (similar amount to `ProcessEdgesWork`).
            // Executing these work packets now can remarkably reduce the global synchronization time.
            self.worker().do_work(work_packet);
        } else {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(work_packet);
        }
    }

    /// Flush the nodes in ProcessEdgesBase, and create a ScanObjects work packet for it. If the node set is empty,
    /// this method will simply return with no work packet created.
    #[cold]
    fn flush(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let scan_objects_work = ScanObjects::<Self>::new(self.pop_nodes(), false);
        self.new_scan_work(scan_objects_work);
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
    default fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ProcessEdgesWork");
        self.set_worker(worker);
        self.process_edges();
        if !self.nodes.is_empty() {
            self.flush();
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
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ScanObjects");
        <E::VM as VMBinding>::VMScanning::scan_objects::<E>(&self.buffer, worker);
        trace!("ScanObjects End");
    }
}

pub struct ProcessModBuf<E: ProcessEdgesWork> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<E>,
    meta: MetadataSpec,
}

impl<E: ProcessEdgesWork> ProcessModBuf<E> {
    pub fn new(modbuf: Vec<ObjectReference>, meta: MetadataSpec) -> Self {
        Self {
            modbuf,
            meta,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessModBuf<E> {
    #[inline(always)]
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        if !self.modbuf.is_empty() {
            for obj in &self.modbuf {
                store_metadata::<E::VM>(&self.meta, *obj, 1, None, Some(Ordering::SeqCst));
            }
        }
        if mmtk.plan.is_current_gc_nursery() {
            if !self.modbuf.is_empty() {
                let mut modbuf = vec![];
                ::std::mem::swap(&mut modbuf, &mut self.modbuf);
                GCWork::do_work(&mut ScanObjects::<E>::new(modbuf, false), worker, mmtk)
            }
        } else {
            // Do nothing
        }
    }
}
