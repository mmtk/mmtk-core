use super::work_bucket::WorkBucketStage;
use super::*;
use crate::plan::GcStatus;
use crate::plan::ObjectsClosure;
use crate::plan::VectorObjectQueue;
use crate::util::*;
use crate::vm::edge_shape::Edge;
use crate::vm::*;
use crate::*;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

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
pub struct Prepare<C: GCWorkContext> {
    pub plan: &'static C::PlanType,
}

impl<C: GCWorkContext> Prepare<C> {
    pub fn new(plan: &'static C::PlanType) -> Self {
        Self { plan }
    }
}

impl<C: GCWorkContext + 'static> GCWork<C::VM> for Prepare<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Prepare Global");
        // We assume this is the only running work packet that accesses plan at the point of execution
        #[allow(clippy::cast_ref_to_mut)]
        let plan_mut: &mut C::PlanType = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.prepare(worker.tls);

        for mutator in <C::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(PrepareMutator::<C::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(PrepareCollector));
            debug_assert!(result.is_ok());
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
pub struct PrepareCollector;

impl<VM: VMBinding> GCWork<VM> for PrepareCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("Prepare Collector");
        worker.get_copy_context_mut().prepare();
        mmtk.plan.prepare_worker(worker);
    }
}

/// The global GC release Work
/// This work packet invokes release() for the plan (which will invoke release() for each space), and
/// pushes work packets for releasing mutators and collectors.
/// We should only have one such work packet per GC, after all actual GC work ends.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Release<C: GCWorkContext> {
    pub plan: &'static C::PlanType,
}

impl<C: GCWorkContext> Release<C> {
    pub fn new(plan: &'static C::PlanType) -> Self {
        Self { plan }
    }
}

impl<C: GCWorkContext + 'static> GCWork<C::VM> for Release<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Release Global");
        <C::VM as VMBinding>::VMCollection::vm_release();
        // We assume this is the only running work packet that accesses plan at the point of execution
        #[allow(clippy::cast_ref_to_mut)]
        let plan_mut: &mut C::PlanType = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.release(worker.tls);

        for mutator in <C::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Release]
                .add(ReleaseMutator::<C::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(ReleaseCollector));
            debug_assert!(result.is_ok());
        }
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
pub struct ReleaseCollector;

impl<VM: VMBinding> GCWork<VM> for ReleaseCollector {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("Release Collector");
        worker.get_copy_context_mut().release();
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
        // If the VM requires that only the coordinator thread can stop the world,
        // we delegate the work to the coordinator.
        if <E::VM as VMBinding>::VMCollection::COORDINATOR_ONLY_STW && !worker.is_coordinator() {
            mmtk.scheduler
                .add_coordinator_work(StopMutators::<E>::new(), worker);
            return;
        }

        trace!("stop_all_mutators start");
        mmtk.plan.base().prepare_for_stack_scanning();
        <E::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls, |mutator| {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanStackRoot::<E>(mutator));
        });
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
        mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanVMSpecificRoots::<E>::new());
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
            mmtk.edge_logger.reset();
        }

        if <VM as VMBinding>::VMCollection::COORDINATOR_ONLY_STW {
            assert!(worker.is_coordinator(),
                    "VM only allows coordinator to resume mutators, but the current worker is not the coordinator.");
        }

        mmtk.plan.base().set_gc_status(GcStatus::NotInGC);

        // Reset the triggering information.
        mmtk.plan.base().reset_collection_trigger();

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
pub struct VMProcessWeakRefs<E: ProcessEdgesWork>(PhantomData<E>);

impl<E: ProcessEdgesWork> VMProcessWeakRefs<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for VMProcessWeakRefs<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ProcessWeakRefs");
        <E::VM as VMBinding>::VMCollection::process_weak_refs(worker); // TODO: Pass a factory/callback to decide what work packet to create.
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
        let factory = ProcessEdgesWorkRootsWorkFactory::<E>::new(mmtk);
        <E::VM as VMBinding>::VMScanning::scan_thread_roots(worker.tls, factory);
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
        let factory = ProcessEdgesWorkRootsWorkFactory::<E>::new(mmtk);
        <E::VM as VMBinding>::VMScanning::scan_thread_root(
            worker.tls,
            unsafe { &mut *(self.0 as *mut _) },
            factory,
        );
        self.0.flush();

        if mmtk.plan.base().inform_stack_scanned(mutators) {
            <E::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(
                false, worker.tls,
            );
            base.set_gc_status(GcStatus::GcProper);
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
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanStaticRoots");
        let factory = ProcessEdgesWorkRootsWorkFactory::<E>::new(mmtk);
        <E::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
    }
}

pub struct ProcessEdgesBase<VM: VMBinding> {
    pub edges: Vec<VM::VMEdge>,
    pub nodes: VectorObjectQueue,
    mmtk: &'static MMTK<VM>,
    // Use raw pointer for fast pointer dereferencing, instead of using `Option<&'static mut GCWorker<E::VM>>`.
    // Because a copying gc will dereference this pointer at least once for every object copy.
    worker: *mut GCWorker<VM>,
    pub roots: bool,
}

unsafe impl<VM: VMBinding> Send for ProcessEdgesBase<VM> {}

impl<VM: VMBinding> ProcessEdgesBase<VM> {
    // Requires an MMTk reference. Each plan-specific type that uses ProcessEdgesBase can get a static plan reference
    // at creation. This avoids overhead for dynamic dispatch or downcasting plan for each object traced.
    pub fn new(edges: Vec<VM::VMEdge>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        #[cfg(feature = "extreme_assertions")]
        if crate::util::edge_logger::should_check_duplicate_edges(&*mmtk.plan) {
            for edge in &edges {
                // log edge, panic if already logged
                mmtk.edge_logger.log_edge(*edge);
            }
        }
        Self {
            edges,
            nodes: VectorObjectQueue::new(),
            mmtk,
            worker: std::ptr::null_mut(),
            roots,
        }
    }
    pub fn set_worker(&mut self, worker: &mut GCWorker<VM>) {
        self.worker = worker;
    }
    #[inline]
    pub fn worker(&self) -> &'static mut GCWorker<VM> {
        unsafe { &mut *self.worker }
    }
    #[inline]
    pub fn mmtk(&self) -> &'static MMTK<VM> {
        self.mmtk
    }
    #[inline]
    pub fn plan(&self) -> &'static dyn Plan<VM = VM> {
        &*self.mmtk.plan
    }
    /// Pop all nodes from nodes, and clear nodes to an empty vector.
    #[inline]
    pub fn pop_nodes(&mut self) -> Vec<ObjectReference> {
        self.nodes.take()
    }
}

/// A short-hand for `<E::VM as VMBinding>::VMEdge`.
pub type EdgeOf<E> = <<E as ProcessEdgesWork>::VM as VMBinding>::VMEdge;

/// Scan & update a list of object slots
//
// Note: be very careful when using this trait. process_node() will push objects
// to the buffer, and it is expected that at the end of the operation, flush()
// is called to create new scan work from the buffered objects. If flush()
// is not called, we may miss the objects in the GC and have dangling pointers.
// FIXME: We possibly want to enforce Drop on this trait, and require calling
// flush() in Drop.
pub trait ProcessEdgesWork:
    Send + 'static + Sized + DerefMut + Deref<Target = ProcessEdgesBase<Self::VM>>
{
    type VM: VMBinding;

    /// The work packet type for scanning objects when using this ProcessEdgesWork.
    type ScanObjectsWorkType: ScanObjectsWork<Self::VM>;

    const CAPACITY: usize = 4096;
    const OVERWRITE_REFERENCE: bool = true;
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;

    fn new(edges: Vec<EdgeOf<Self>>, roots: bool, mmtk: &'static MMTK<Self::VM>) -> Self;

    /// Trace an MMTk object. The implementation should forward this call to the policy-specific
    /// `trace_object()` methods, depending on which space this object is in.
    /// If the object is not in any MMTk space, the implementation should forward the call to
    /// `ActivePlan::vm_trace_object()` to let the binding handle the tracing.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;

    #[cfg(feature = "sanity")]
    fn cache_roots_for_sanity_gc(&mut self) {
        assert!(self.roots);
        self.mmtk()
            .sanity_checker
            .lock()
            .unwrap()
            .add_roots(self.edges.clone());
    }

    /// Start the a scan work packet. If SCAN_OBJECTS_IMMEDIATELY, the work packet will be executed immediately, in this method.
    /// Otherwise, the work packet will be added the Closure work bucket and will be dispatched later by the scheduler.
    #[inline]
    fn start_or_dispatch_scan_work(&mut self, work_packet: impl GCWork<Self::VM>) {
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

    /// Create an object-scanning work packet to be used for this ProcessEdgesWork.
    ///
    /// `roots` indicates if we are creating a packet for root scanning.  It is only true when this
    /// method is called to handle `RootsWorkFactory::create_process_node_roots_work`.
    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        roots: bool,
    ) -> Self::ScanObjectsWorkType;

    /// Flush the nodes in ProcessEdgesBase, and create a ScanObjects work packet for it. If the node set is empty,
    /// this method will simply return with no work packet created.
    #[cold]
    fn flush(&mut self) {
        let nodes = self.pop_nodes();
        if !nodes.is_empty() {
            self.start_or_dispatch_scan_work(self.create_scan_work(nodes, false));
        }
    }

    #[inline]
    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        let new_object = self.trace_object(object);
        if Self::OVERWRITE_REFERENCE {
            slot.store(new_object);
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
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("ProcessEdgesWork");
        self.set_worker(worker);
        self.process_edges();
        if !self.nodes.is_empty() {
            self.flush();
        }
        #[cfg(feature = "sanity")]
        if self.roots {
            self.cache_roots_for_sanity_gc();
        }
        trace!("ProcessEdgesWork End");
    }
}

/// A general process edges implementation using SFT. A plan can always implement their own process edges. However,
/// Most plans can use this work packet for tracing amd they do not need to provide a plan-specific trace object work packet.
/// If they choose to use this type, they need to provide a correct implementation for some related methods
/// (such as `Space.set_copy_for_sft_trace()`, `SFT.sft_trace_object()`).
/// Some plans are not using this type, mostly due to more complex tracing. Either it is impossible to use this type, or
/// there is performance overheads for using this general trace type. In such cases, they implement their specific process edges.
// TODO: This is not used any more. Should we remove it?
pub struct SFTProcessEdges<VM: VMBinding> {
    pub base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for SFTProcessEdges<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;

    fn new(edges: Vec<EdgeOf<Self>>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        Self { base }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        use crate::policy::sft::GCWorkerMutRef;
        use crate::policy::sft_map::SFTMap;

        if object.is_null() {
            return object;
        }

        // Erase <VM> type parameter
        let worker = GCWorkerMutRef::new(self.worker());

        // Invoke trace object on sft
        let sft = unsafe { crate::mmtk::SFT_MAP.get_unchecked(object.to_address::<VM>()) };
        sft.sft_trace_object(&mut self.base.nodes, object, worker)
    }

    #[inline(always)]
    fn create_scan_work(&self, nodes: Vec<ObjectReference>, roots: bool) -> ScanObjects<Self> {
        ScanObjects::<Self>::new(nodes, false, roots)
    }
}

struct ProcessEdgesWorkRootsWorkFactory<E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
}

impl<E: ProcessEdgesWork> Clone for ProcessEdgesWorkRootsWorkFactory<E> {
    fn clone(&self) -> Self {
        Self { mmtk: self.mmtk }
    }
}

impl<E: ProcessEdgesWork> RootsWorkFactory<EdgeOf<E>> for ProcessEdgesWorkRootsWorkFactory<E> {
    fn create_process_edge_roots_work(&mut self, edges: Vec<EdgeOf<E>>) {
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::Closure,
            E::new(edges, true, self.mmtk),
        );
    }

    fn create_process_node_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        // Note: Node roots cannot be moved.  Currently, this implies that the plan must never
        // move objects.  However, in the future, if we start to support object pinning, then
        // moving plans that support object pinning (such as Immix) can still use node roots.
        assert!(
            !self.mmtk.plan.constraints().moves_objects,
            "Attempted to add node roots when using a plan that moves objects.  Plan: {:?}",
            *self.mmtk.options.plan
        );

        // We want to use E::create_scan_work.
        let process_edges_work = E::new(vec![], true, self.mmtk);
        let work = process_edges_work.create_scan_work(nodes, true);
        crate::memory_manager::add_work_packet(self.mmtk, WorkBucketStage::Closure, work);
    }
}

impl<E: ProcessEdgesWork> ProcessEdgesWorkRootsWorkFactory<E> {
    fn new(mmtk: &'static MMTK<E::VM>) -> Self {
        Self { mmtk }
    }
}

impl<VM: VMBinding> Deref for SFTProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for SFTProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// Trait for a work packet that scans objects
pub trait ScanObjectsWork<VM: VMBinding>: GCWork<VM> + Sized {
    /// The associated ProcessEdgesWork for processing the edges of the objects in this packet.
    type E: ProcessEdgesWork<VM = VM>;

    /// Return true if the objects in this packet are pointed by roots, in which case we need to
    /// call trace_object on them.
    fn roots(&self) -> bool;

    /// Called after each object is scanned.
    fn post_scan_object(&self, object: ObjectReference);

    /// Create another object-scanning work packet of the same kind, to scan adjacent objects of
    /// the objects in this packet.
    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self;

    /// The common code for ScanObjects and PlanScanObjects.
    fn do_work_common(
        &self,
        buffer: &[ObjectReference],
        worker: &mut GCWorker<<Self::E as ProcessEdgesWork>::VM>,
        mmtk: &'static MMTK<<Self::E as ProcessEdgesWork>::VM>,
    ) {
        let tls = worker.tls;

        // If this is a root packet, the objects in this packet will have not been traced, yet.
        //
        // This step conceptually traces the edges from root slots to the objects they point to.
        // However, VMs that deliver root objects instead of root edges are incapable of updating
        // root slots.  Like processing an edge, we call `trace_object` on those objects, and
        // assert the GC doesn't move those objects because we cannot store back to the slots.
        //
        // If this is a root packet, the `scanned_root_objects` variable will hold those root
        // objects which are traced for the first time.
        let scanned_root_objects = self.roots().then(|| {
            // We create an instance of E to use its `trace_object` method and its object queue.
            let mut process_edges_work = Self::E::new(vec![], false, mmtk);

            for object in buffer.iter().copied() {
                let new_object = process_edges_work.trace_object(object);
                debug_assert_eq!(
                    object, new_object,
                    "Object moved while tracing root unmovable root object: {} -> {}",
                    object, new_object
                );
            }

            // This contains root objects that are visited the first time.
            // It is sufficient to only scan these objects.
            process_edges_work.nodes.take()
        });

        // If it is a root packet, scan the nodes that are first scanned;
        // otherwise, scan the nodes in the buffer.
        let objects_to_scan = scanned_root_objects.as_deref().unwrap_or(buffer);

        // Then scan those objects for edges.
        let mut scan_later = vec![];
        {
            let mut closure = ObjectsClosure::<Self::E>::new(worker);
            for object in objects_to_scan.iter().copied() {
                if <VM as VMBinding>::VMScanning::support_edge_enqueuing(tls, object) {
                    // If an object supports edge-enqueuing, we enqueue its edges.
                    <VM as VMBinding>::VMScanning::scan_object(tls, object, &mut closure);
                    self.post_scan_object(object);
                } else {
                    // If an object does not support edge-enqueuing, we have to use
                    // `Scanning::scan_object_and_trace_edges` and offload the job of updating the
                    // reference field to the VM.
                    //
                    // However, at this point, `closure` is borrowing `worker`.
                    // So we postpone the processing of objects that needs object enqueuing
                    scan_later.push(object);
                }
            }
        }

        // If any object does not support edge-enqueuing, we process them now.
        if !scan_later.is_empty() {
            // We create an instance of E to use its `trace_object` method and its object queue.
            let mut process_edges_work = Self::E::new(vec![], false, mmtk);
            let mut closure = |object| process_edges_work.trace_object(object);

            // Scan objects and trace their edges at the same time.
            for object in scan_later.iter().copied() {
                <VM as VMBinding>::VMScanning::scan_object_and_trace_edges(
                    tls,
                    object,
                    &mut closure,
                );
                self.post_scan_object(object);
            }

            // Create work packets to scan adjacent objects.  We skip ProcessEdgesWork and create
            // object-scanning packets directly, because the edges are already traced.
            if !process_edges_work.nodes.is_empty() {
                let next_nodes = process_edges_work.nodes.take();
                let make_packet = |nodes| {
                    let work_packet = self.make_another(nodes);
                    memory_manager::add_work_packet(mmtk, WorkBucketStage::Closure, work_packet);
                };

                // Divide the resulting nodes into appropriately sized packets.
                if next_nodes.len() <= Self::E::CAPACITY {
                    make_packet(next_nodes);
                } else {
                    for chunk in next_nodes.chunks(Self::E::CAPACITY) {
                        make_packet(chunk.into());
                    }
                }
            }
        }
    }
}

/// Scan objects and enqueue the edges of the objects.  For objects that do not support
/// edge-enqueuing, this work packet also processes the edges.
///
/// This work packet does not execute policy-specific post-scanning hooks
/// (it won't call `post_scan_object()` in [`policy::gc_work::PolicyTraceObject`]).
/// It should be used only for policies that do not perform policy-specific actions when scanning
/// an object.
pub struct ScanObjects<Edges: ProcessEdgesWork> {
    buffer: Vec<ObjectReference>,
    #[allow(unused)]
    concurrent: bool,
    roots: bool,
    phantom: PhantomData<Edges>,
}

impl<Edges: ProcessEdgesWork> ScanObjects<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool, roots: bool) -> Self {
        Self {
            buffer,
            concurrent,
            roots,
            phantom: PhantomData,
        }
    }
}

impl<VM: VMBinding, E: ProcessEdgesWork<VM = VM>> ScanObjectsWork<VM> for ScanObjects<E> {
    type E = E;

    fn roots(&self) -> bool {
        self.roots
    }

    #[inline(always)]
    fn post_scan_object(&self, _object: ObjectReference) {
        // Do nothing.
    }

    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self {
        Self::new(buffer, self.concurrent, false)
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanObjects<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanObjects");
        self.do_work_common(&self.buffer, worker, mmtk);
        trace!("ScanObjects End");
    }
}

use crate::mmtk::MMTK;
use crate::plan::Plan;
use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;

/// This provides an implementation of [`ProcessEdgesWork`](scheduler/gc_work/ProcessEdgesWork). A plan that implements
/// `PlanTraceObject` can use this work packet for tracing objects.
pub struct PlanProcessEdges<
    VM: VMBinding,
    P: Plan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> ProcessEdgesWork
    for PlanProcessEdges<VM, P, KIND>
{
    type VM = VM;
    type ScanObjectsWorkType = PlanScanObjects<Self, P>;

    fn new(edges: Vec<EdgeOf<Self>>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        Self { plan, base }
    }

    #[inline(always)]
    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        roots: bool,
    ) -> Self::ScanObjectsWorkType {
        PlanScanObjects::<Self, P>::new(self.plan, nodes, false, roots)
    }

    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // We cannot borrow `self` twice in a call, so we extract `worker` as a local variable.
        let worker = self.worker();
        self.plan
            .trace_object::<VectorObjectQueue, KIND>(&mut self.base.nodes, object, worker)
    }

    #[inline]
    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        let new_object = self.trace_object(object);
        if P::may_move_objects::<KIND>() {
            slot.store(new_object);
        }
    }
}

// Impl Deref/DerefMut to ProcessEdgesBase for PlanProcessEdges
impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> Deref
    for PlanProcessEdges<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> DerefMut
    for PlanProcessEdges<VM, P, KIND>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// This is an alternative to `ScanObjects` that calls the `post_scan_object` of the policy
/// selected by the plan.  It is applicable to plans that derive `PlanTraceObject`.
pub struct PlanScanObjects<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> {
    plan: &'static P,
    buffer: Vec<ObjectReference>,
    #[allow(dead_code)]
    concurrent: bool,
    roots: bool,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> PlanScanObjects<E, P> {
    pub fn new(
        plan: &'static P,
        buffer: Vec<ObjectReference>,
        concurrent: bool,
        roots: bool,
    ) -> Self {
        Self {
            plan,
            buffer,
            concurrent,
            roots,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> ScanObjectsWork<E::VM>
    for PlanScanObjects<E, P>
{
    type E = E;

    fn roots(&self) -> bool {
        self.roots
    }

    #[inline(always)]
    fn post_scan_object(&self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }

    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self {
        Self::new(self.plan, buffer, self.concurrent, false)
    }
}

impl<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> GCWork<E::VM>
    for PlanScanObjects<E, P>
{
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("PlanScanObjects");
        self.do_work_common(&self.buffer, worker, mmtk);
        trace!("PlanScanObjects End");
    }
}
