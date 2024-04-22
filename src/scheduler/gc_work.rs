use super::work_bucket::WorkBucketStage;
use super::*;
use crate::global_state::GcStatus;
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
        // Tell GC trigger that GC started.
        mmtk.gc_trigger.policy.on_gc_start(mmtk);

        // Determine collection kind
        let is_emergency = mmtk.state.set_collection_kind(
            mmtk.get_plan().last_collection_was_exhaustive(),
            mmtk.gc_trigger.policy.can_heap_size_grow(),
        );
        if is_emergency {
            mmtk.get_plan().notify_emergency_collection();
        }
        // Set to GcPrepare
        mmtk.set_gc_status(GcStatus::GcPrepare);

        // Let the plan to schedule collection work
        mmtk.get_plan().schedule_collection(worker.scheduler());
    }
}

/// The global GC Preparation Work
/// This work packet invokes prepare() for the plan (which will invoke prepare() for each space), and
/// pushes work packets for preparing mutators and collectors.
/// We should only have one such work packet per GC, before any actual GC work starts.
/// We assume this work packet is the only running work packet that accesses plan, and there should
/// be no other concurrent work packet that accesses plan (read or write). Otherwise, there may
/// be a race condition.
pub struct Prepare<C: GCWorkContext> {
    pub plan: *const C::PlanType,
}

unsafe impl<C: GCWorkContext> Send for Prepare<C> {}

impl<C: GCWorkContext> Prepare<C> {
    pub fn new(plan: *const C::PlanType) -> Self {
        Self { plan }
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for Prepare<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Prepare Global");
        // We assume this is the only running work packet that accesses plan at the point of execution
        let plan_mut: &mut C::PlanType = unsafe { &mut *(self.plan as *const _ as *mut _) };
        plan_mut.prepare(worker.tls);

        if plan_mut.constraints().needs_prepare_mutator {
            for mutator in <C::VM as VMBinding>::VMActivePlan::mutators() {
                mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                    .add(PrepareMutator::<C::VM>::new(mutator));
            }
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
        mmtk.get_plan().prepare_worker(worker);
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
    pub plan: *const C::PlanType,
}

impl<C: GCWorkContext> Release<C> {
    pub fn new(plan: *const C::PlanType) -> Self {
        Self { plan }
    }
}

unsafe impl<C: GCWorkContext> Send for Release<C> {}

impl<C: GCWorkContext + 'static> GCWork<C::VM> for Release<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("Release Global");

        mmtk.gc_trigger.policy.on_gc_release(mmtk);
        // We assume this is the only running work packet that accesses plan at the point of execution

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

        #[cfg(feature = "count_live_bytes_in_gc")]
        {
            let live_bytes = mmtk
                .scheduler
                .worker_group
                .get_and_clear_worker_live_bytes();
            mmtk.state.set_live_bytes_in_last_gc(live_bytes);
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
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<C: GCWorkContext>(PhantomData<C>);

impl<C: GCWorkContext> StopMutators<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for StopMutators<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("stop_all_mutators start");
        mmtk.state.prepare_for_stack_scanning();
        <C::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls, |mutator| {
            // TODO: The stack scanning work won't start immediately, as the `Prepare` bucket is not opened yet (the bucket is opened in notify_mutators_paused).
            // Should we push to Unconstrained instead?
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(ScanMutatorRoots::<C>(mutator));
        });
        trace!("stop_all_mutators end");
        mmtk.scheduler.notify_mutators_paused(mmtk);
        mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanVMSpecificRoots::<C>::new());
    }
}

/// This implements `ObjectTracer` by forwarding the `trace_object` calls to the wrapped
/// `ProcessEdgesWork` instance.
pub(crate) struct ProcessEdgesWorkTracer<E: ProcessEdgesWork> {
    process_edges_work: E,
    stage: WorkBucketStage,
}

impl<E: ProcessEdgesWork> ObjectTracer for ProcessEdgesWorkTracer<E> {
    /// Forward the `trace_object` call to the underlying `ProcessEdgesWork`,
    /// and flush as soon as the underlying buffer of `process_edges_work` is full.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        debug_assert!(!object.is_null());
        let result = self.process_edges_work.trace_object(object);
        self.flush_if_full();
        result
    }
}

impl<E: ProcessEdgesWork> ProcessEdgesWorkTracer<E> {
    fn flush_if_full(&mut self) {
        if self.process_edges_work.nodes.is_full() {
            self.flush();
        }
    }

    pub fn flush_if_not_empty(&mut self) {
        if !self.process_edges_work.nodes.is_empty() {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let next_nodes = self.process_edges_work.pop_nodes();
        assert!(!next_nodes.is_empty());
        let work_packet = self.process_edges_work.create_scan_work(next_nodes);
        let worker = self.process_edges_work.worker();
        worker.scheduler().work_buckets[self.stage].add(work_packet);
    }
}

/// This type implements `ObjectTracerContext` by creating a temporary `ProcessEdgesWork` during
/// the call to `with_tracer`, making use of its `trace_object` method.  It then creates work
/// packets using the methods of the `ProcessEdgesWork` and add the work packet into the given
/// `stage`.
pub(crate) struct ProcessEdgesWorkTracerContext<E: ProcessEdgesWork> {
    stage: WorkBucketStage,
    phantom_data: PhantomData<E>,
}

impl<E: ProcessEdgesWork> Clone for ProcessEdgesWorkTracerContext<E> {
    fn clone(&self) -> Self {
        Self { ..*self }
    }
}

impl<E: ProcessEdgesWork> ObjectTracerContext<E::VM> for ProcessEdgesWorkTracerContext<E> {
    type TracerType = ProcessEdgesWorkTracer<E>;

    fn with_tracer<R, F>(&self, worker: &mut GCWorker<E::VM>, func: F) -> R
    where
        F: FnOnce(&mut Self::TracerType) -> R,
    {
        let mmtk = worker.mmtk;

        // Prepare the underlying ProcessEdgesWork
        let mut process_edges_work = E::new(vec![], false, mmtk, self.stage);
        // FIXME: This line allows us to omit the borrowing lifetime of worker.
        // We should refactor ProcessEdgesWork so that it uses `worker` locally, not as a member.
        process_edges_work.set_worker(worker);

        // Cretae the tracer.
        let mut tracer = ProcessEdgesWorkTracer {
            process_edges_work,
            stage: self.stage,
        };

        // The caller can use the tracer here.
        let result = func(&mut tracer);

        // Flush the queued nodes.
        tracer.flush_if_not_empty();

        result
    }
}

/// Delegate to the VM binding for weak reference processing.
///
/// Some VMs (e.g. v8) do not have a Java-like global weak reference storage, and the
/// processing of those weakrefs may be more complex. For such case, we delegate to the
/// VM binding to process weak references.
///
/// NOTE: This will replace `{Soft,Weak,Phantom}RefProcessing` and `Finalization` in the future.
pub struct VMProcessWeakRefs<E: ProcessEdgesWork> {
    phantom_data: PhantomData<E>,
}

impl<E: ProcessEdgesWork> VMProcessWeakRefs<E> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for VMProcessWeakRefs<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("VMProcessWeakRefs");

        let stage = WorkBucketStage::VMRefClosure;

        let need_to_repeat = {
            let tracer_factory = ProcessEdgesWorkTracerContext::<E> {
                stage,
                phantom_data: PhantomData,
            };
            <E::VM as VMBinding>::VMScanning::process_weak_refs(worker, tracer_factory)
        };

        if need_to_repeat {
            // Schedule Self as the new sentinel so we'll call `process_weak_refs` again after the
            // current transitive closure.
            let new_self = Box::new(Self::new());

            worker.scheduler().work_buckets[stage].set_sentinel(new_self);
        }
    }
}

/// Delegate to the VM binding for forwarding weak references.
///
/// Some VMs (e.g. v8) do not have a Java-like global weak reference storage, and the
/// processing of those weakrefs may be more complex. For such case, we delegate to the
/// VM binding to process weak references.
///
/// NOTE: This will replace `RefForwarding` and `ForwardFinalization` in the future.
pub struct VMForwardWeakRefs<E: ProcessEdgesWork> {
    phantom_data: PhantomData<E>,
}

impl<E: ProcessEdgesWork> VMForwardWeakRefs<E> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for VMForwardWeakRefs<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        trace!("VMForwardWeakRefs");

        let stage = WorkBucketStage::VMRefForwarding;

        let tracer_factory = ProcessEdgesWorkTracerContext::<E> {
            stage,
            phantom_data: PhantomData,
        };
        <E::VM as VMBinding>::VMScanning::forward_weak_refs(worker, tracer_factory)
    }
}

/// This work packet calls `Collection::post_forwarding`.
///
/// NOTE: This will replace `RefEnqueue` in the future.
///
/// NOTE: Although this work packet runs in parallel with the `Release` work packet, it does not
/// access the `Plan` instance.
#[derive(Default)]
pub struct VMPostForwarding<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> GCWork<VM> for VMPostForwarding<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        trace!("VMPostForwarding start");
        <VM as VMBinding>::VMCollection::post_forwarding(worker.tls);
        trace!("VMPostForwarding end");
    }
}

pub struct ScanMutatorRoots<C: GCWorkContext>(pub &'static mut Mutator<C::VM>);

impl<C: GCWorkContext> GCWork<C::VM> for ScanMutatorRoots<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("ScanMutatorRoots for mutator {:?}", self.0.get_tls());
        let mutators = <C::VM as VMBinding>::VMActivePlan::number_of_mutators();
        let factory = ProcessEdgesWorkRootsWorkFactory::<
            C::VM,
            C::DefaultProcessEdges,
            C::PinningProcessEdges,
        >::new(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_roots_in_mutator_thread(
            worker.tls,
            unsafe { &mut *(self.0 as *mut _) },
            factory,
        );
        self.0.flush();

        if mmtk.state.inform_stack_scanned(mutators) {
            <C::VM as VMBinding>::VMScanning::notify_initial_thread_scan_complete(
                false, worker.tls,
            );
            mmtk.set_gc_status(GcStatus::GcProper);
        }
    }
}

#[derive(Default)]
pub struct ScanVMSpecificRoots<C: GCWorkContext>(PhantomData<C>);

impl<C: GCWorkContext> ScanVMSpecificRoots<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for ScanVMSpecificRoots<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("ScanStaticRoots");
        let factory = ProcessEdgesWorkRootsWorkFactory::<
            C::VM,
            C::DefaultProcessEdges,
            C::PinningProcessEdges,
        >::new(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
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
    pub bucket: WorkBucketStage,
}

unsafe impl<VM: VMBinding> Send for ProcessEdgesBase<VM> {}

impl<VM: VMBinding> ProcessEdgesBase<VM> {
    // Requires an MMTk reference. Each plan-specific type that uses ProcessEdgesBase can get a static plan reference
    // at creation. This avoids overhead for dynamic dispatch or downcasting plan for each object traced.
    pub fn new(
        edges: Vec<VM::VMEdge>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        #[cfg(feature = "extreme_assertions")]
        if crate::util::edge_logger::should_check_duplicate_edges(mmtk.get_plan()) {
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
            bucket,
        }
    }
    pub fn set_worker(&mut self, worker: &mut GCWorker<VM>) {
        self.worker = worker;
    }

    pub fn worker(&self) -> &'static mut GCWorker<VM> {
        unsafe { &mut *self.worker }
    }

    pub fn mmtk(&self) -> &'static MMTK<VM> {
        self.mmtk
    }

    pub fn plan(&self) -> &'static dyn Plan<VM = VM> {
        self.mmtk.get_plan()
    }

    /// Pop all nodes from nodes, and clear nodes to an empty vector.
    pub fn pop_nodes(&mut self) -> Vec<ObjectReference> {
        self.nodes.take()
    }

    pub fn is_roots(&self) -> bool {
        self.roots
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
    /// The associate type for the VM.
    type VM: VMBinding;

    /// The work packet type for scanning objects when using this ProcessEdgesWork.
    type ScanObjectsWorkType: ScanObjectsWork<Self::VM>;

    /// The maximum number of edges that should be put to one of this work packets.
    /// The caller who creates a work packet of this trait should be responsible to
    /// comply with this capacity.
    /// Higher capacity means the packet will take longer to finish, and may lead to
    /// bad load balancing. On the other hand, lower capacity would lead to higher cost
    /// on scheduling many small work packets. It is important to find a proper capacity.
    const CAPACITY: usize = 4096;
    /// Do we update object reference? This has to be true for a moving GC.
    const OVERWRITE_REFERENCE: bool = true;
    /// If true, we do object scanning in this work packet with the same worker without scheduling overhead.
    /// If false, we will add object scanning work packets to the global queue and allow other workers to work on it.
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;

    /// Create a [`ProcessEdgesWork`].
    ///
    /// Arguments:
    /// * `edges`: a vector of the edges.
    /// * `roots`: are the objects root reachable objects?
    /// * `mmtk`: a reference to the MMTK instance.
    /// * `bucket`: which work bucket this packet belongs to. Further work generated from this packet will also be put to the same bucket.
    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self;

    /// Trace an MMTk object. The implementation should forward this call to the policy-specific
    /// `trace_object()` methods, depending on which space this object is in.
    /// If the object is not in any MMTk space, the implementation should forward the call to
    /// `ActivePlan::vm_trace_object()` to let the binding handle the tracing.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;

    /// If the work includes roots, we will store the roots somewhere so for sanity GC, we can do another
    /// transitive closure from the roots.
    #[cfg(feature = "sanity")]
    fn cache_roots_for_sanity_gc(&mut self) {
        assert!(self.roots);
        self.mmtk()
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_edges(self.edges.clone());
    }

    /// Start the a scan work packet. If SCAN_OBJECTS_IMMEDIATELY, the work packet will be executed immediately, in this method.
    /// Otherwise, the work packet will be added the Closure work bucket and will be dispatched later by the scheduler.
    fn start_or_dispatch_scan_work(&mut self, mut work_packet: impl GCWork<Self::VM>) {
        if Self::SCAN_OBJECTS_IMMEDIATELY {
            // We execute this `scan_objects_work` immediately.
            // This is expected to be a useful optimization because,
            // say for _pmd_ with 200M heap, we're likely to have 50000~60000 `ScanObjects` work packets
            // being dispatched (similar amount to `ProcessEdgesWork`).
            // Executing these work packets now can remarkably reduce the global synchronization time.
            work_packet.do_work(self.worker(), self.mmtk);
        } else {
            debug_assert!(self.bucket != WorkBucketStage::Unconstrained);
            self.mmtk.scheduler.work_buckets[self.bucket].add(work_packet);
        }
    }

    /// Create an object-scanning work packet to be used for this ProcessEdgesWork.
    ///
    /// `roots` indicates if we are creating a packet for root scanning.  It is only true when this
    /// method is called to handle `RootsWorkFactory::create_process_pinning_roots_work`.
    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType;

    /// Flush the nodes in ProcessEdgesBase, and create a ScanObjects work packet for it. If the node set is empty,
    /// this method will simply return with no work packet created.
    fn flush(&mut self) {
        let nodes = self.pop_nodes();
        if !nodes.is_empty() {
            self.start_or_dispatch_scan_work(self.create_scan_work(nodes));
        }
    }

    /// Process an edge, including loading the object reference from the memory slot,
    /// trace the object and store back the new object reference if necessary.
    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        if object.is_null() {
            return;
        }
        let new_object = self.trace_object(object);
        debug_assert!(!new_object.is_null());
        if Self::OVERWRITE_REFERENCE && new_object != object {
            slot.store(new_object);
        }
    }

    /// Process all the edges in the work packet.
    fn process_edges(&mut self) {
        probe!(mmtk, process_edges, self.edges.len(), self.is_roots());
        for i in 0..self.edges.len() {
            self.process_edge(self.edges[i])
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for E {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, _mmtk: &'static MMTK<E::VM>) {
        self.set_worker(worker);
        self.process_edges();
        if !self.nodes.is_empty() {
            self.flush();
        }
        #[cfg(feature = "sanity")]
        if self.roots && !_mmtk.is_in_sanity() {
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

    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk, bucket);
        Self { base }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        use crate::policy::sft::GCWorkerMutRef;

        debug_assert!(!object.is_null());

        // Erase <VM> type parameter
        let worker = GCWorkerMutRef::new(self.worker());

        // Invoke trace object on sft
        let sft = unsafe { crate::mmtk::SFT_MAP.get_unchecked(object.to_address::<VM>()) };
        sft.sft_trace_object(&mut self.base.nodes, object, worker)
    }

    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> ScanObjects<Self> {
        ScanObjects::<Self>::new(nodes, false, self.bucket)
    }
}

/// An implementation of `RootsWorkFactory` that creates work packets based on `ProcessEdgesWork`
/// for handling roots.  The `DPE` and the `PPE` type parameters correspond to the
/// `DefaultProcessEdge` and the `PinningProcessEdges` type members of the [`GCWorkContext`] trait.
pub(crate) struct ProcessEdgesWorkRootsWorkFactory<
    VM: VMBinding,
    DPE: ProcessEdgesWork<VM = VM>,
    PPE: ProcessEdgesWork<VM = VM>,
> {
    mmtk: &'static MMTK<VM>,
    phantom: PhantomData<(DPE, PPE)>,
}

impl<VM: VMBinding, DPE: ProcessEdgesWork<VM = VM>, PPE: ProcessEdgesWork<VM = VM>> Clone
    for ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn clone(&self) -> Self {
        Self {
            mmtk: self.mmtk,
            phantom: PhantomData,
        }
    }
}

impl<VM: VMBinding, DPE: ProcessEdgesWork<VM = VM>, PPE: ProcessEdgesWork<VM = VM>>
    RootsWorkFactory<VM::VMEdge> for ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn create_process_edge_roots_work(&mut self, edges: Vec<VM::VMEdge>) {
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::Closure,
            DPE::new(edges, true, self.mmtk, WorkBucketStage::Closure),
        );
    }

    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        // Will process roots within the PinningRootsTrace bucket
        // And put work in the Closure bucket
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::PinningRootsTrace,
            ProcessRootNode::<VM, PPE, DPE>::new(nodes, WorkBucketStage::Closure),
        );
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::TPinningClosure,
            ProcessRootNode::<VM, PPE, PPE>::new(nodes, WorkBucketStage::TPinningClosure),
        );
    }
}

impl<VM: VMBinding, DPE: ProcessEdgesWork<VM = VM>, PPE: ProcessEdgesWork<VM = VM>>
    ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            phantom: PhantomData,
        }
    }
}

impl<VM: VMBinding> Deref for SFTProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for SFTProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// Trait for a work packet that scans objects
pub trait ScanObjectsWork<VM: VMBinding>: GCWork<VM> + Sized {
    /// The associated ProcessEdgesWork for processing the edges of the objects in this packet.
    type E: ProcessEdgesWork<VM = VM>;

    /// Called after each object is scanned.
    fn post_scan_object(&self, object: ObjectReference);

    /// Create another object-scanning work packet of the same kind, to scan adjacent objects of
    /// the objects in this packet.
    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self;

    fn get_bucket(&self) -> WorkBucketStage;

    /// The common code for ScanObjects and PlanScanObjects.
    fn do_work_common(
        &self,
        buffer: &[ObjectReference],
        worker: &mut GCWorker<<Self::E as ProcessEdgesWork>::VM>,
        _mmtk: &'static MMTK<<Self::E as ProcessEdgesWork>::VM>,
    ) {
        let tls = worker.tls;

        // Scan the nodes in the buffer.
        let objects_to_scan = buffer;

        // Then scan those objects for edges.
        let mut scan_later = vec![];
        {
            let mut closure = ObjectsClosure::<Self::E>::new(worker, self.get_bucket());
            for object in objects_to_scan.iter().copied() {
                // For any object we need to scan, we count its liv bytes
                #[cfg(feature = "count_live_bytes_in_gc")]
                closure
                    .worker
                    .shared
                    .increase_live_bytes(VM::VMObjectModel::get_current_size(object));

                if <VM as VMBinding>::VMScanning::support_edge_enqueuing(tls, object) {
                    trace!("Scan object (edge) {}", object);
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
            let object_tracer_context = ProcessEdgesWorkTracerContext::<Self::E> {
                stage: self.get_bucket(),
                phantom_data: PhantomData,
            };

            object_tracer_context.with_tracer(worker, |object_tracer| {
                // Scan objects and trace their edges at the same time.
                for object in scan_later.iter().copied() {
                    trace!("Scan object (node) {}", object);
                    <VM as VMBinding>::VMScanning::scan_object_and_trace_edges(
                        tls,
                        object,
                        object_tracer,
                    );
                    self.post_scan_object(object);
                }
            });
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
    phantom: PhantomData<Edges>,
    bucket: WorkBucketStage,
}

impl<Edges: ProcessEdgesWork> ScanObjects<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool, bucket: WorkBucketStage) -> Self {
        Self {
            buffer,
            concurrent,
            phantom: PhantomData,
            bucket,
        }
    }
}

impl<VM: VMBinding, E: ProcessEdgesWork<VM = VM>> ScanObjectsWork<VM> for ScanObjects<E> {
    type E = E;

    fn get_bucket(&self) -> WorkBucketStage {
        self.bucket
    }

    fn post_scan_object(&self, _object: ObjectReference) {
        // Do nothing.
    }

    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self {
        Self::new(buffer, self.concurrent, self.bucket)
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

/// This provides an implementation of [`crate::scheduler::gc_work::ProcessEdgesWork`]. A plan that implements
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

    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk, bucket);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        Self { plan, base }
    }

    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType {
        PlanScanObjects::<Self, P>::new(self.plan, nodes, false, self.bucket)
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        debug_assert!(!object.is_null());
        // We cannot borrow `self` twice in a call, so we extract `worker` as a local variable.
        let worker = self.worker();
        self.plan
            .trace_object::<VectorObjectQueue, KIND>(&mut self.base.nodes, object, worker)
    }

    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        if object.is_null() {
            return;
        }
        let new_object = self.trace_object(object);
        debug_assert!(!new_object.is_null());
        if P::may_move_objects::<KIND>() && new_object != object {
            slot.store(new_object);
        }
    }
}

// Impl Deref/DerefMut to ProcessEdgesBase for PlanProcessEdges
impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> Deref
    for PlanProcessEdges<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> DerefMut
    for PlanProcessEdges<VM, P, KIND>
{
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
    phantom: PhantomData<E>,
    bucket: WorkBucketStage,
}

impl<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> PlanScanObjects<E, P> {
    pub fn new(
        plan: &'static P,
        buffer: Vec<ObjectReference>,
        concurrent: bool,
        bucket: WorkBucketStage,
    ) -> Self {
        Self {
            plan,
            buffer,
            concurrent,
            phantom: PhantomData,
            bucket,
        }
    }
}

impl<E: ProcessEdgesWork, P: Plan<VM = E::VM> + PlanTraceObject<E::VM>> ScanObjectsWork<E::VM>
    for PlanScanObjects<E, P>
{
    type E = E;

    fn get_bucket(&self) -> WorkBucketStage {
        self.bucket
    }

    fn post_scan_object(&self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }

    fn make_another(&self, buffer: Vec<ObjectReference>) -> Self {
        Self::new(self.plan, buffer, self.concurrent, self.bucket)
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

/// This work packet processes pinning roots.
///
/// The `roots` member holds a list of `ObjectReference` to objects directly pointed by roots.
/// These objects will be traced using `R2OPE` (Root-to-Object Process Edges).
///
/// After that, it will create work packets for tracing their children.  Those work packets (and
/// the work packets further created by them) will use `O2OPE` (Object-to-Object Process Edges) as
/// their `ProcessEdgesWork` implementations.
///
/// Because `roots` are pinning roots, `R2OPE` must be a `ProcessEdgesWork` that never moves any
/// object.
///
/// The choice of `O2OPE` determines whether the `roots` are transitively pinning or not.
///
/// -   If `O2OPE` is set to a `ProcessEdgesWork` that never moves objects, all descendents of
///     `roots` will not be moved in this GC.  That implements transitive pinning roots.
/// -   If `O2OPE` may move objects, then this `ProcessRootsNode<VM, R2OPE, O2OPE>` work packet
///     will only pin the objects in `roots` (because `R2OPE` must not move objects anyway), but
///     not their descendents.
pub(crate) struct ProcessRootNode<
    VM: VMBinding,
    R2OPE: ProcessEdgesWork<VM = VM>,
    O2OPE: ProcessEdgesWork<VM = VM>,
> {
    phantom: PhantomData<(VM, R2OPE, O2OPE)>,
    roots: Vec<ObjectReference>,
    bucket: WorkBucketStage,
}

impl<VM: VMBinding, R2OPE: ProcessEdgesWork<VM = VM>, O2OPE: ProcessEdgesWork<VM = VM>>
    ProcessRootNode<VM, R2OPE, O2OPE>
{
    pub fn new(nodes: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            phantom: PhantomData,
            roots: nodes,
            bucket,
        }
    }
}

impl<VM: VMBinding, R2OPE: ProcessEdgesWork<VM = VM>, O2OPE: ProcessEdgesWork<VM = VM>> GCWork<VM>
    for ProcessRootNode<VM, R2OPE, O2OPE>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("ProcessRootNode");

        #[cfg(feature = "sanity")]
        {
            if !mmtk.is_in_sanity() {
                mmtk.sanity_checker
                    .lock()
                    .unwrap()
                    .add_root_nodes(self.roots.clone());
            }
        }

        // Because this is a root packet, the objects in this packet will have not been traced, yet.
        //
        // This step conceptually traces the edges from root slots to the objects they point to.
        // However, VMs that deliver root objects instead of root edges are incapable of updating
        // root slots.  Like processing an edge, we call `trace_object` on those objects, and
        // assert the GC doesn't move those objects because we cannot store back to the slots.
        //
        // The `scanned_root_objects` variable will hold those root
        // objects which are traced for the first time and we create work for scanning those roots.
        let scanned_root_objects = {
            // We create an instance of E to use its `trace_object` method and its object queue.
            let mut process_edges_work =
                R2OPE::new(vec![], true, mmtk, WorkBucketStage::PinningRootsTrace);
            process_edges_work.set_worker(worker);

            for object in self.roots.iter().copied() {
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
        };

        let process_edges_work = O2OPE::new(vec![], false, mmtk, self.bucket);
        let work = process_edges_work.create_scan_work(scanned_root_objects);
        crate::memory_manager::add_work_packet(mmtk, self.bucket, work);

        trace!("ProcessRootNode End");
    }
}

/// A `ProcessEdgesWork` type that panics when any of its method is used.
/// This is currently used for plans that do not support transitively pinning.
#[derive(Default)]
pub struct UnsupportedProcessEdges<VM: VMBinding> {
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> Deref for UnsupportedProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        panic!("unsupported!")
    }
}

impl<VM: VMBinding> DerefMut for UnsupportedProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        panic!("unsupported!")
    }
}

impl<VM: VMBinding> ProcessEdgesWork for UnsupportedProcessEdges<VM> {
    type VM = VM;

    type ScanObjectsWorkType = ScanObjects<Self>;

    fn new(
        _edges: Vec<EdgeOf<Self>>,
        _roots: bool,
        _mmtk: &'static MMTK<Self::VM>,
        _bucket: WorkBucketStage,
    ) -> Self {
        panic!("unsupported!")
    }

    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        panic!("unsupported!")
    }

    fn create_scan_work(&self, _nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType {
        panic!("unsupported!")
    }
}
