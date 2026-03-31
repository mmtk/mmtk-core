use super::work_bucket::WorkBucketStage;
use super::*;
use crate::global_state::GcStatus;
use crate::plan::tracing::Trace;
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

/// This implementation of [`ObjectTracer`] queues newly visited objects and create the
/// [`TracingProcessNodes`] work packets to scan and trace objects.
pub(crate) struct TracingObjectTracer<'w, T: Trace> {
    worker: &'w mut GCWorker<T::VM>,
    policy: T,
    queue: VectorObjectQueue,
    stage: WorkBucketStage,
}

impl<'w, T: Trace> ObjectTracer for TracingObjectTracer<'w, T> {
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

impl<'w, T: Trace> TracingObjectTracer<'w, T> {
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
        let work_packet = TracingProcessNodes::new(self.policy.clone(), next_nodes, self.stage);
        self.worker.scheduler().work_buckets[self.stage].add(work_packet);
    }
}

/// This implementation of [`ObjectTracerContext`] creates the [`TracingObjectTracer`] to expand the
/// transitive closure during a stop-the-world tracing GC or the final mark pause of a concurrent
/// GC.  It is used during object scanning as well as weak reference processing.
pub(crate) struct TracingTracerContext<T: Trace> {
    policy: T,
    stage: WorkBucketStage,
}

impl<T: Trace> TracingTracerContext<T> {
    pub fn new(policy: T, stage: WorkBucketStage) -> Self {
        Self { policy, stage }
    }
}

impl<T: Trace> Clone for TracingTracerContext<T> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            stage: self.stage,
        }
    }
}

impl<T: Trace> ObjectTracerContext<T::VM> for TracingTracerContext<T> {
    type TracerType<'w> = TracingObjectTracer<'w, T>;

    fn with_tracer<'w, R, F>(&self, worker: &'w mut GCWorker<T::VM>, func: F) -> R
    where
        F: FnOnce(&mut Self::TracerType<'w>) -> R,
    {
        let mmtk = worker.mmtk;

        // Cretae the callback tracer.
        let mut tracer = TracingObjectTracer::new(worker, T::from_mmtk(mmtk), self.stage);

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
pub struct VMProcessWeakRefs<T: Trace> {
    phantom_data: PhantomData<T>,
}

impl<T: Trace> VMProcessWeakRefs<T> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<T: Trace> GCWork<T::VM> for VMProcessWeakRefs<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("VMProcessWeakRefs");

        let stage = WorkBucketStage::VMRefClosure;

        let need_to_repeat = {
            let tracer_factory = TracingTracerContext::new(T::from_mmtk(mmtk), stage);
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
pub struct VMForwardWeakRefs<T: Trace> {
    phantom_data: PhantomData<T>,
}

impl<T: Trace> VMForwardWeakRefs<T> {
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<T: Trace> GCWork<T::VM> for VMForwardWeakRefs<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("VMForwardWeakRefs");

        let stage = WorkBucketStage::VMRefForwarding;

        let tracer_factory = TracingTracerContext::new(T::from_mmtk(mmtk), stage);
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
        let factory = TracingRootsWorkFactory::<C::VM, C::DefaultTrace, C::PinningTrace>::new(mmtk);
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
        let factory = TracingRootsWorkFactory::<C::VM, C::DefaultTrace, C::PinningTrace>::new(mmtk);
        <C::VM as VMBinding>::VMScanning::scan_vm_specific_roots(worker.tls, factory);
    }
}

/// A short-hand for `<E::VM as VMBinding>::VMSlot`.
pub type SlotOfTP<E> = <<E as Trace>::VM as VMBinding>::VMSlot;

/// A work packet for processing slots during a stop-the-world tracing GC and the final mark pause
/// of a concurrent GC.
///
/// It will call `trace_object` on the value of each slot, and updates the slot if the object is
/// moved or forwarded.  It will spawn or immediately run the [`DefaultScanObjects`] work packet to
/// scan newly traced objects.
pub struct TracingProcessSlots<T: Trace> {
    policy: T,
    slots: Vec<SlotOfTP<T>>,
    #[allow(unused)] // Only used by sanity
    roots: bool,
    bucket: WorkBucketStage,
}

impl<T: Trace> TracingProcessSlots<T> {
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

impl<T: Trace> GCWork<T::VM> for TracingProcessSlots<T> {
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
            let mut work =
                TracingProcessNodes::new(self.policy.clone(), queued_objects, self.bucket);

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

/// An implementation of [`RootsWorkFactory`] for stop-the-world tracing GC, i.e. finding the
/// transitive closure from roots, with all mutators stopped.
///
/// It will create relevant work packets for tpinning, pinning and non-pinning roots, and put them
/// into the stop-the-world work buckets.
pub(crate) struct TracingRootsWorkFactory<VM: VMBinding, DPE: Trace<VM = VM>, PPE: Trace<VM = VM>> {
    pub(crate) mmtk: &'static MMTK<VM>,
    phantom: PhantomData<(DPE, PPE)>,
}

impl<VM: VMBinding, DPE: Trace<VM = VM>, PPE: Trace<VM = VM>> Clone
    for TracingRootsWorkFactory<VM, DPE, PPE>
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
pub(crate) enum RootsKind {
    NORMAL = 0,
    PINNING = 1,
    TPINNING = 2,
}

impl<VM: VMBinding, DPE: Trace<VM = VM>, PPE: Trace<VM = VM>> RootsWorkFactory<VM::VMSlot>
    for TracingRootsWorkFactory<VM, DPE, PPE>
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
            TracingProcessSlots::new(
                DPE::from_mmtk(self.mmtk),
                slots,
                true,
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
            TracingProcessPinningRoots::<VM, PPE, DPE>::new(nodes, WorkBucketStage::Closure),
        );
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::TPINNING, nodes.len());
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::TPinningClosure,
            TracingProcessPinningRoots::<VM, PPE, PPE>::new(
                nodes,
                WorkBucketStage::TPinningClosure,
            ),
        );
    }
}

impl<VM: VMBinding, DPE: Trace<VM = VM>, PPE: Trace<VM = VM>>
    TracingRootsWorkFactory<VM, DPE, PPE>
{
    pub(crate) fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            phantom: PhantomData,
        }
    }
}

/// A work packet for scanning objects and optionally do node-enqueuing tracing during a
/// stop-the-world tracing GC and the final mark pause of a concurrent GC.
///
/// It will scan each objects.  For objects that supports slot enqueuing, it will collect their
/// slots and spawn [`TracingProcessSlots`] work packets to trace them.  For objects that don't
/// support slot enqueuing, it will immediately trace their slots and spawn other
/// [`TracingProcessNodes`] work packets to process their newly traced children.
pub struct TracingProcessNodes<T: Trace> {
    policy: T,
    objects: Vec<ObjectReference>,
    bucket: WorkBucketStage,
}

impl<T: Trace> TracingProcessNodes<T> {
    pub fn new(policy: T, objects: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            policy,
            objects,
            bucket,
        }
    }
}

impl<T: Trace> GCWork<T::VM> for TracingProcessNodes<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("ScanObjects");

        let tls = worker.tls;

        // Scan the objects in the list that supports slot-enququing.
        let mut scan_later = vec![];
        {
            let mut slots = Vec::new();

            let flush = |slots: &mut _, worker: &mut GCWorker<T::VM>| {
                let buffer = std::mem::take(slots);
                let work_packet =
                    TracingProcessSlots::new(T::from_mmtk(mmtk), buffer, false, self.bucket);
                worker.add_work(self.bucket, work_packet);
            };

            // For any object we need to scan, we count its live bytes.
            // Check the option outside the loop for better performance.
            if crate::util::rust_util::unlikely(*mmtk.get_options().count_live_bytes_in_gc) {
                // Borrow before the loop.
                let mut live_bytes_stats = worker.shared.live_bytes_per_space.borrow_mut();
                for object in self.objects.iter().copied() {
                    crate::scheduler::worker::GCWorkerShared::<T::VM>::increase_live_bytes(
                        &mut live_bytes_stats,
                        object,
                    );
                }
            }

            for object in self.objects.iter().copied() {
                if <T::VM as VMBinding>::VMScanning::support_slot_enqueuing(tls, object) {
                    trace!("Scan object (slot) {}", object);
                    // If an object supports slot-enqueuing, we enqueue its slots.
                    <T::VM as VMBinding>::VMScanning::scan_object(tls, object, &mut |slot| {
                        slots.push(slot);
                        if slots.len() >= EDGES_WORK_BUFFER_SIZE {
                            flush(&mut slots, worker);
                        }
                    });
                    self.policy.post_scan_object(object);
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

            if !slots.is_empty() {
                flush(&mut slots, worker);
            }
        }

        let total_objects = self.objects.len();
        let scan_and_trace = scan_later.len();
        probe!(mmtk, scan_objects, total_objects, scan_and_trace);

        // If any object does not support slot-enqueuing, we process them now.
        if !scan_later.is_empty() {
            let object_tracer_context = TracingTracerContext::new(self.policy.clone(), self.bucket);

            object_tracer_context.with_tracer(worker, |object_tracer| {
                // Scan objects and trace their outgoing edges at the same time.
                for object in scan_later.iter().copied() {
                    trace!("Scan object (node) {}", object);
                    <T::VM as VMBinding>::VMScanning::scan_object_and_trace_edges(
                        tls,
                        object,
                        object_tracer,
                    );
                    self.policy.post_scan_object(object);
                }
            });
        }
        trace!("ScanObjects End");
    }
}

/// This work packet processes pinning roots during stop-the-world tracing GC.
///
/// Note that by definition, a "root" is an *edge* that points from outside the object graph.  This
/// work packet represents each edge as the `ObjectReference` of the object the edge points to.
/// Because pinning roots by definition cannot be updated, we don't need to represent the edges with
/// [`Slot`].
///
/// The `roots` member holds a list of `ObjectReference` to objects directly pointed by roots. These
/// objects will be traced using `R2OTP` (Root-to-Object Trace Policy).
///
/// After that, it will create work packets for tracing their children.  Those work packets (and the
/// work packets further created by them) will use `O2OPE` (Object-to-Object Trace Policy) as their
/// `Trace` implementations.
///
/// Because `roots` are pinning roots, `R2OTP` must be a `Trace` that never moves any object.
///
/// The choice of `O2OPE` determines whether the `roots` are transitively pinning or not.
///
/// -   If `O2OPE` is set to a `Trace` that never moves objects, no descendents of `roots` will be
///     moved in this GC.  That implements transitive pinning roots.
/// -   If `O2OPE` may move objects, then this `ProcessRootsNode<VM, R2OTP, O2OPE>` work packet will
///     only pin the objects in `roots` (because `R2OTP` must not move objects anyway), but not
///     their descendents.
pub(crate) struct TracingProcessPinningRoots<
    VM: VMBinding,
    R2OTP: Trace<VM = VM>,
    O2OTP: Trace<VM = VM>,
> {
    phantom: PhantomData<(VM, R2OTP, O2OTP)>,
    roots: Vec<ObjectReference>,
    bucket: WorkBucketStage,
}

impl<VM: VMBinding, R2OTP: Trace<VM = VM>, O2OTP: Trace<VM = VM>>
    TracingProcessPinningRoots<VM, R2OTP, O2OTP>
{
    pub fn new(nodes: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            phantom: PhantomData,
            roots: nodes,
            bucket,
        }
    }
}

impl<VM: VMBinding, R2OTP: Trace<VM = VM>, O2OTP: Trace<VM = VM>> GCWork<VM>
    for TracingProcessPinningRoots<VM, R2OTP, O2OTP>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("TracingProcessPinningRoots");

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
                TracingProcessNodes::new(O2OTP::from_mmtk(mmtk), root_objects_to_scan, self.bucket);
            worker.add_work(self.bucket, work);
        }

        trace!("TracingProcessPinningRoots End");
    }
}
