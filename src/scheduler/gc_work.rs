use super::work_bucket::WorkBucketStage;
use super::*;
use crate::global_state::GcStatus;
use crate::plan::tracing::PlanTracePolicy;
use crate::plan::tracing::SFTTracePolicy;
use crate::plan::tracing::TracePolicy;
use crate::plan::ObjectsClosure;
use crate::plan::VectorObjectQueue;
use crate::util::*;
use crate::vm::slot::Slot;
use crate::vm::*;
use crate::*;
use std::marker::PhantomData;

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
            let prepare_mutator_packets = <C::VM as VMBinding>::VMActivePlan::mutators()
                .map(|mutator| Box::new(PrepareMutator::<C::VM>::new(mutator)) as _)
                .collect::<Vec<_>>();
            // Just in case the VM binding is inconsistent about the number of mutators and the actual mutator list.
            debug_assert_eq!(
                prepare_mutator_packets.len(),
                <C::VM as VMBinding>::VMActivePlan::number_of_mutators()
            );
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].bulk_add(prepare_mutator_packets);
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

        let release_mutator_packets = <C::VM as VMBinding>::VMActivePlan::mutators()
            .map(|mutator| Box::new(ReleaseMutator::<C::VM>::new(mutator)) as _)
            .collect::<Vec<_>>();
        // Just in case the VM binding is inconsistent about the number of mutators and the actual mutator list.
        debug_assert_eq!(
            release_mutator_packets.len(),
            <C::VM as VMBinding>::VMActivePlan::number_of_mutators()
        );
        mmtk.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(release_mutator_packets);

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
/// TODO: Smaller work granularity
#[derive(Default)]
pub struct StopMutators<C: GCWorkContext> {
    /// If this is true, we skip creating [`ScanMutatorRoots`] work packets for mutators.
    /// By default, this is false.
    skip_mutator_roots: bool,
    /// Flush mutators once they are stopped. By default this is false. [`ScanMutatorRoots`] will flush mutators.
    flush_mutator: bool,
    phantom: PhantomData<C>,
}

impl<C: GCWorkContext> StopMutators<C> {
    pub fn new() -> Self {
        Self {
            skip_mutator_roots: false,
            flush_mutator: false,
            phantom: PhantomData,
        }
    }

    /// Create a `StopMutators` work packet that does not create `ScanMutatorRoots` work packets for mutators, and will simply flush mutators.
    pub fn new_no_scan_roots() -> Self {
        Self {
            skip_mutator_roots: true,
            flush_mutator: true,
            phantom: PhantomData,
        }
    }
}

impl<C: GCWorkContext> GCWork<C::VM> for StopMutators<C> {
    fn do_work(&mut self, worker: &mut GCWorker<C::VM>, mmtk: &'static MMTK<C::VM>) {
        trace!("stop_all_mutators start");
        mmtk.state.prepare_for_stack_scanning();
        <C::VM as VMBinding>::VMCollection::stop_all_mutators(worker.tls, |mutator| {
            // TODO: The stack scanning work won't start immediately, as the `Prepare` bucket is not opened yet (the bucket is opened in notify_mutators_paused).
            // Should we push to Unconstrained instead?

            if self.flush_mutator {
                mutator.flush();
            }
            if !self.skip_mutator_roots {
                mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                    .add(ScanMutatorRoots::<C>(mutator));
            }
        });
        trace!("stop_all_mutators end");
        mmtk.get_plan().notify_mutators_paused(&mmtk.scheduler);
        mmtk.scheduler.notify_mutators_paused(mmtk);
        mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].add(ScanVMSpecificRoots::<C>::new());
    }
}

/// This implementation of `ObjectTracer` queues newly visited objects and create object-scanning work packets.
pub(crate) struct QueuingObjectTracer<'w, T: TracePolicy> {
    worker: &'w mut GCWorker<T::VM>,
    policy: T,
    queue: VectorObjectQueue,
    stage: WorkBucketStage,
}

impl<'w, T: TracePolicy> ObjectTracer for QueuingObjectTracer<'w, T> {
    /// Forward the `trace_object` call to the underlying `ProcessEdgesWork`,
    /// and flush as soon as the underlying buffer of `process_edges_work` is full.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        let result = self
            .policy
            .trace_object(self.worker, object, &mut self.queue);
        self.flush_if_full();
        result
    }
}

impl<'w, T: TracePolicy> QueuingObjectTracer<'w, T> {
    fn new(worker: &'w mut GCWorker<T::VM>, policy: T, stage: WorkBucketStage) -> Self {
        Self {
            worker,
            policy,
            queue: VectorObjectQueue::new(),
            stage,
        }
    }

    fn flush_if_full(&mut self) {
        if self.queue.is_full() {
            self.flush();
        }
    }

    pub fn flush_if_not_empty(&mut self) {
        if !self.queue.is_empty() {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let next_nodes = self.queue.take();
        assert!(!next_nodes.is_empty());
        let work_packet = self
            .policy
            .create_scan_work(next_nodes, self.worker.mmtk, self.stage);
        self.worker.scheduler().work_buckets[self.stage].add(work_packet);
    }
}

/// This is the default implementation of [`ObjectTracerContext`]. It creates the
/// [`QueuingObjectTracer`] which queues newly visited objects and creates work packets for scanning
/// objects.
pub(crate) struct DefaultTracerContext<T: TracePolicy> {
    policy: T,
    stage: WorkBucketStage,
}

impl<T: TracePolicy> DefaultTracerContext<T> {
    pub fn new(policy: T, stage: WorkBucketStage) -> Self {
        Self { policy, stage }
    }
}

impl<T: TracePolicy> Clone for DefaultTracerContext<T> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            stage: self.stage,
        }
    }
}

impl<T: TracePolicy> ObjectTracerContext<T::VM> for DefaultTracerContext<T> {
    type TracerType<'w> = QueuingObjectTracer<'w, T>;

    fn with_tracer<'w, R, F>(&self, worker: &'w mut GCWorker<T::VM>, func: F) -> R
    where
        F: FnOnce(&mut Self::TracerType<'w>) -> R,
    {
        let mmtk = worker.mmtk;

        // Cretae the callback tracer.
        let mut tracer = QueuingObjectTracer::new(worker, T::from_mmtk(mmtk), self.stage);

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
pub struct VMProcessWeakRefs<T: TracePolicy> {
    phantom_data: PhantomData<T>,
}

impl<T: TracePolicy> VMProcessWeakRefs<T> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<T: TracePolicy> GCWork<T::VM> for VMProcessWeakRefs<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("VMProcessWeakRefs");

        let stage = WorkBucketStage::VMRefClosure;

        let need_to_repeat = {
            let tracer_factory = DefaultTracerContext::new(T::from_mmtk(mmtk), stage);
            <T::VM as VMBinding>::VMScanning::process_weak_refs(worker, tracer_factory)
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
pub struct VMForwardWeakRefs<T: TracePolicy> {
    phantom_data: PhantomData<T>,
}

impl<T: TracePolicy> VMForwardWeakRefs<T> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<T: TracePolicy> GCWork<T::VM> for VMForwardWeakRefs<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("VMForwardWeakRefs");

        let stage = WorkBucketStage::VMRefForwarding;

        let tracer_factory = DefaultTracerContext::new(T::from_mmtk(mmtk), stage);
        <T::VM as VMBinding>::VMScanning::forward_weak_refs(worker, tracer_factory)
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
            C::DefaultTracePolicy,
            C::PinningTracePolicy,
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
            C::DefaultTracePolicy,
            C::PinningTracePolicy,
        >::new(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
    }
}

/// A short-hand for `<E::VM as VMBinding>::VMSlot`.
pub type SlotOfTP<E> = <<E as TracePolicy>::VM as VMBinding>::VMSlot;

pub struct DefaultProcessSlots<T: TracePolicy> {
    policy: T,
    slots: Vec<SlotOfTP<T>>,
    #[allow(unused)] // Only used by sanity
    roots: bool,
    bucket: WorkBucketStage,
}

impl<T: TracePolicy> DefaultProcessSlots<T> {
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;

    pub fn new(policy: T, slots: Vec<SlotOfTP<T>>, roots: bool, bucket: WorkBucketStage) -> Self {
        Self {
            policy,
            slots,
            roots,
            bucket,
        }
    }

    /// If the work includes roots, we will store the roots somewhere so for sanity GC, we can do another
    /// transitive closure from the roots.
    #[cfg(feature = "sanity")]
    fn cache_roots_for_sanity_gc(&mut self) {
        assert!(self.roots);
        self.mmtk()
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_slots(self.slots.clone());
    }
}

impl<T: TracePolicy> GCWork<T::VM> for DefaultProcessSlots<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        let mut queue = VectorObjectQueue::new();

        for slot in self.slots.iter() {
            if let Some(object) = slot.load() {
                let new_object = self.policy.trace_object(worker, object, &mut queue);
                if T::may_move_objects() && new_object != object {
                    slot.store(new_object);
                }
            }
        }

        if !queue.is_empty() {
            let queued_objects = queue.take();
            let mut work = self
                .policy
                .create_scan_work(queued_objects, mmtk, self.bucket);

            if Self::SCAN_OBJECTS_IMMEDIATELY {
                work.do_work(worker, mmtk);
            } else {
                worker.add_work(self.bucket, work);
            }
        }

        #[cfg(feature = "sanity")]
        if self.roots && !mmtk.is_in_sanity() {
            self.cache_roots_for_sanity_gc();
        }
    }
}

/// A general implementation of [`ProcessEdgesWork`] using SFT. A plan can always implement their
/// own [`ProcessEdgesWork`] instances. However, most plans can use this work packet for tracing amd
/// they do not need to provide a plan-specific trace object work packet. If they choose to use this
/// type, they need to provide a correct implementation for some related methods (such as
/// `Space.set_copy_for_sft_trace()`, `SFT.sft_trace_object()`). Some plans are not using this type,
/// mostly due to more complex tracing. Either it is impossible to use this type, or there is
/// performance overheads for using this general trace type. In such cases, they implement their
/// specific [`ProcessEdgesWork`] instances.
// TODO: This is not used any more. Should we remove it?
#[allow(dead_code)]
pub struct SFTProcessSlots<VM: VMBinding> {
    base: DefaultProcessSlots<SFTTracePolicy<VM>>,
}

impl<VM: VMBinding> SFTProcessSlots<VM> {
    pub fn new(
        slots: Vec<VM::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let policy = SFTTracePolicy::from_mmtk(mmtk);
        let base = DefaultProcessSlots::new(policy, slots, roots, bucket);
        Self { base }
    }
}

impl<VM: VMBinding> GCWork<VM> for SFTProcessSlots<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        self.base.do_work(worker, mmtk);
    }
}

/// An implementation of `RootsWorkFactory` that creates work packets based on `ProcessEdgesWork`
/// for handling roots.  The `DPE` and the `PPE` type parameters correspond to the
/// `DefaultProcessEdge` and the `PinningProcessEdges` type members of the [`GCWorkContext`] trait.
pub(crate) struct ProcessEdgesWorkRootsWorkFactory<
    VM: VMBinding,
    DPE: TracePolicy<VM = VM>,
    PPE: TracePolicy<VM = VM>,
> {
    mmtk: &'static MMTK<VM>,
    phantom: PhantomData<(DPE, PPE)>,
}

impl<VM: VMBinding, DPE: TracePolicy<VM = VM>, PPE: TracePolicy<VM = VM>> Clone
    for ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn clone(&self) -> Self {
        Self {
            mmtk: self.mmtk,
            phantom: PhantomData,
        }
    }
}

/// For USDT tracepoints for roots.
/// Keep in sync with `tools/tracing/timeline/visualize.py`.
#[repr(usize)]
enum RootsKind {
    NORMAL = 0,
    PINNING = 1,
    TPINNING = 2,
}

impl<VM: VMBinding, DPE: TracePolicy<VM = VM>, PPE: TracePolicy<VM = VM>>
    RootsWorkFactory<VM::VMSlot> for ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn create_process_roots_work(&mut self, slots: Vec<VM::VMSlot>) {
        // Note: We should use the same USDT name "mmtk:roots" for all the three kinds of roots. A
        // VM binding may not call all of the three methods in this impl. For example, the OpenJDK
        // binding only calls `create_process_roots_work`, and the Ruby binding only calls
        // `create_process_pinning_roots_work`. Because `ProcessEdgesWorkRootsWorkFactory<VM, DPE,
        // PPE>` is a generic type, the Rust compiler emits the function bodies on demand, so the
        // resulting machine code may not contain all three USDT trace points.  If they have
        // different names, and our `capture.bt` mentions all of them, `bpftrace` may complain that
        // it cannot find one or more of those USDT trace points in the binary.
        probe!(mmtk, roots, RootsKind::NORMAL, slots.len());
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::Closure,
            DPE::from_mmtk(self.mmtk).make_process_slots_work(
                slots,
                true,
                self.mmtk,
                WorkBucketStage::Closure,
            ),
        );
    }

    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::PINNING, nodes.len());
        // Will process roots within the PinningRootsTrace bucket
        // And put work in the Closure bucket
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::PinningRootsTrace,
            ProcessRootNodes::<VM, PPE, DPE>::new(nodes, WorkBucketStage::Closure),
        );
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::TPINNING, nodes.len());
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::TPinningClosure,
            ProcessRootNodes::<VM, PPE, PPE>::new(nodes, WorkBucketStage::TPinningClosure),
        );
    }
}

impl<VM: VMBinding, DPE: TracePolicy<VM = VM>, PPE: TracePolicy<VM = VM>>
    ProcessEdgesWorkRootsWorkFactory<VM, DPE, PPE>
{
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            phantom: PhantomData,
        }
    }
}

/// Trait for a work packet that scans objects
pub trait ScanObjectsWork<VM: VMBinding>: GCWork<VM> + Sized {
    /// The associated ProcessEdgesWork for processing the outgoing edges of the objects in this
    /// packet.
    type T: TracePolicy<VM = VM>;

    /// Called after each object is scanned.
    fn post_scan_object(&self, object: ObjectReference);

    /// Return the work bucket for this work packet and its derived work packets.
    fn get_bucket(&self) -> WorkBucketStage;

    /// The common code for ScanObjects and PlanScanObjects.
    fn do_work_common(
        &self,
        buffer: &[ObjectReference],
        worker: &mut GCWorker<<Self::T as TracePolicy>::VM>,
        mmtk: &'static MMTK<<Self::T as TracePolicy>::VM>,
    ) {
        let tls = worker.tls;

        let objects_to_scan = buffer;

        // Scan the objects in the list that supports slot-enququing.
        let mut scan_later = vec![];
        {
            let mut closure = ObjectsClosure::<Self::T>::new(worker, self.get_bucket());

            // For any object we need to scan, we count its live bytes.
            // Check the option outside the loop for better performance.
            if crate::util::rust_util::unlikely(*mmtk.get_options().count_live_bytes_in_gc) {
                // Borrow before the loop.
                let mut live_bytes_stats = closure.worker.shared.live_bytes_per_space.borrow_mut();
                for object in objects_to_scan.iter().copied() {
                    crate::scheduler::worker::GCWorkerShared::<VM>::increase_live_bytes(
                        &mut live_bytes_stats,
                        object,
                    );
                }
            }

            for object in objects_to_scan.iter().copied() {
                if <VM as VMBinding>::VMScanning::support_slot_enqueuing(tls, object) {
                    trace!("Scan object (slot) {}", object);
                    // If an object supports slot-enqueuing, we enqueue its slots.
                    <VM as VMBinding>::VMScanning::scan_object(tls, object, &mut closure);
                    self.post_scan_object(object);
                } else {
                    // If an object does not support slot-enqueuing, we have to use
                    // `Scanning::scan_object_and_trace_edges` and offload the job of updating the
                    // reference field to the VM.
                    //
                    // However, at this point, `closure` is borrowing `worker`.
                    // So we postpone the processing of objects that needs object enqueuing
                    scan_later.push(object);
                }
            }
        }

        let total_objects = objects_to_scan.len();
        let scan_and_trace = scan_later.len();
        probe!(mmtk, scan_objects, total_objects, scan_and_trace);

        // If any object does not support slot-enqueuing, we process them now.
        if !scan_later.is_empty() {
            let object_tracer_context =
                DefaultTracerContext::new(Self::T::from_mmtk(mmtk), self.get_bucket());

            object_tracer_context.with_tracer(worker, |object_tracer| {
                // Scan objects and trace their outgoing edges at the same time.
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

/// Scan objects and enqueue the slots of the objects.  For objects that do not support
/// slot-enqueuing, this work packet also traces their outgoing edges directly.
///
/// This work packet does not execute policy-specific post-scanning hooks
/// (it won't call `post_scan_object()` in [`policy::gc_work::PolicyTraceObject`]).
/// It should be used only for policies that do not perform policy-specific actions when scanning
/// an object.
pub struct ScanObjects<T: TracePolicy> {
    buffer: Vec<ObjectReference>,
    #[allow(unused)]
    concurrent: bool,
    phantom: PhantomData<T>,
    bucket: WorkBucketStage,
}

impl<T: TracePolicy> ScanObjects<T> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool, bucket: WorkBucketStage) -> Self {
        Self {
            buffer,
            concurrent,
            phantom: PhantomData,
            bucket,
        }
    }
}

impl<VM: VMBinding, T: TracePolicy<VM = VM>> ScanObjectsWork<VM> for ScanObjects<T> {
    type T = T;

    fn get_bucket(&self) -> WorkBucketStage {
        self.bucket
    }

    fn post_scan_object(&self, _object: ObjectReference) {
        // Do nothing.
    }
}

impl<T: TracePolicy> GCWork<T::VM> for ScanObjects<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
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
pub struct PlanProcessSlots<
    VM: VMBinding,
    P: Plan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    base: DefaultProcessSlots<PlanTracePolicy<P, KIND>>,
}

impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind>
    PlanProcessSlots<VM, P, KIND>
{
    pub fn new(
        slots: Vec<VM::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let policy = PlanTracePolicy::from_mmtk(mmtk);
        let base = DefaultProcessSlots::new(policy, slots, roots, bucket);
        Self { base }
    }
}

impl<VM: VMBinding, P: PlanTraceObject<VM> + Plan<VM = VM>, const KIND: TraceKind> GCWork<VM>
    for PlanProcessSlots<VM, P, KIND>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        self.base.do_work(worker, mmtk);
    }
}

/// This is an alternative to `ScanObjects` that calls the `post_scan_object` of the policy
/// selected by the plan.  It is applicable to plans that derive `PlanTraceObject`.
pub struct PlanScanObjects<T: TracePolicy, P: Plan<VM = T::VM> + PlanTraceObject<T::VM>> {
    plan: &'static P,
    buffer: Vec<ObjectReference>,
    #[allow(dead_code)]
    concurrent: bool,
    phantom: PhantomData<T>,
    bucket: WorkBucketStage,
}

impl<T: TracePolicy, P: Plan<VM = T::VM> + PlanTraceObject<T::VM>> PlanScanObjects<T, P> {
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

impl<T: TracePolicy, P: Plan<VM = T::VM> + PlanTraceObject<T::VM>> ScanObjectsWork<T::VM>
    for PlanScanObjects<T, P>
{
    type T = T;

    fn get_bucket(&self) -> WorkBucketStage {
        self.bucket
    }

    fn post_scan_object(&self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }
}

impl<T: TracePolicy, P: Plan<VM = T::VM> + PlanTraceObject<T::VM>> GCWork<T::VM>
    for PlanScanObjects<T, P>
{
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("PlanScanObjects");
        self.do_work_common(&self.buffer, worker, mmtk);
        trace!("PlanScanObjects End");
    }
}

/// This work packet processes pinning roots.
///
/// The `roots` member holds a list of `ObjectReference` to objects directly pointed by roots. These
/// objects will be traced using `R2OTP` (Root-to-Object Trace Policy).
///
/// After that, it will create work packets for tracing their children.  Those work packets (and the
/// work packets further created by them) will use `O2OPE` (Object-to-Object Trace Policy) as their
/// `TracePolicy` implementations.
///
/// Because `roots` are pinning roots, `R2OTP` must be a `TracePolicy` that never moves any object.
///
/// The choice of `O2OPE` determines whether the `roots` are transitively pinning or not.
///
/// -   If `O2OPE` is set to a `TracePolicy` that never moves objects, no descendents of `roots`
///     will be moved in this GC.  That implements transitive pinning roots.
/// -   If `O2OPE` may move objects, then this `ProcessRootsNode<VM, R2OTP, O2OPE>` work packet will
///     only pin the objects in `roots` (because `R2OTP` must not move objects anyway), but not
///     their descendents.
pub(crate) struct ProcessRootNodes<
    VM: VMBinding,
    R2OTP: TracePolicy<VM = VM>,
    O2OTP: TracePolicy<VM = VM>,
> {
    phantom: PhantomData<(VM, R2OTP, O2OTP)>,
    roots: Vec<ObjectReference>,
    bucket: WorkBucketStage,
}

impl<VM: VMBinding, R2OTP: TracePolicy<VM = VM>, O2OTP: TracePolicy<VM = VM>>
    ProcessRootNodes<VM, R2OTP, O2OTP>
{
    pub fn new(nodes: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            phantom: PhantomData,
            roots: nodes,
            bucket,
        }
    }
}

impl<VM: VMBinding, R2OTP: TracePolicy<VM = VM>, O2OTP: TracePolicy<VM = VM>> GCWork<VM>
    for ProcessRootNodes<VM, R2OTP, O2OTP>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("ProcessRootNodes");

        #[cfg(feature = "sanity")]
        {
            if !mmtk.is_in_sanity() {
                mmtk.sanity_checker
                    .lock()
                    .unwrap()
                    .add_root_nodes(self.roots.clone());
            }
        }

        let num_roots = self.roots.len();

        // This step conceptually traces the edges from root slots to the objects they point to.
        // However, VMs that deliver root objects instead of root slots are incapable of updating
        // root slots.  Therefore, we call `trace_object` on those objects, and assert the GC
        // doesn't move those objects because we cannot store the updated references back to the
        // slots.
        //
        // The `root_objects_to_scan` variable will hold those root objects which are traced for the
        // first time.  We will create a work packet for scanning those roots.
        let root_objects_to_scan = {
            let mut queue = VectorObjectQueue::new();

            let mut r2o_policy = R2OTP::from_mmtk(mmtk);

            for object in self.roots.iter().copied() {
                let new_object = r2o_policy.trace_object(worker, object, &mut queue);
                debug_assert_eq!(
                    object, new_object,
                    "Object moved while tracing root unmovable root object: {} -> {}",
                    object, new_object
                );
            }

            queue.take()
        };

        let num_enqueued_nodes = root_objects_to_scan.len();
        probe!(mmtk, process_root_nodes, num_roots, num_enqueued_nodes);

        if !root_objects_to_scan.is_empty() {
            let work =
                O2OTP::from_mmtk(mmtk).create_scan_work(root_objects_to_scan, mmtk, self.bucket);
            if O2OTP::is_concurrent() {
                worker.scheduler().work_buckets[self.bucket].add_no_notify(work);
            } else {
                worker.add_work(self.bucket, work);
            }
        }

        trace!("ProcessRootNodes End");
    }
}

/// A `ProcessEdgesWork` type that panics when any of its method is used.
/// This is currently used for plans that do not support transitively pinning.
#[derive(Default)]
pub struct UnsupportedProcessEdges<VM: VMBinding> {
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> GCWork<VM> for UnsupportedProcessEdges<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        panic!("unsupported!")
    }
}
