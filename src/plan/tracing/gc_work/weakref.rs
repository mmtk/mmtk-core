use std::marker::PhantomData;

use crate::{
    plan::{
        tracing::{gc_work::closure::ProcessNodes, Trace},
        VectorObjectQueue,
    },
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    util::ObjectReference,
    vm::{Collection, ObjectTracer, ObjectTracerContext, Scanning, VMBinding},
    MMTK,
};

/// This implementation of [`ObjectTracer`] queues newly visited objects and create the
/// [`ProcessNodes`] work packets to scan and trace objects.
pub(crate) struct DefaultObjectTracer<'w, T: Trace> {
    worker: &'w mut GCWorker<T::VM>,
    trace: T,
    queue: VectorObjectQueue,
    stage: WorkBucketStage,
}

impl<T: Trace> ObjectTracer for DefaultObjectTracer<'_, T> {
    /// Forward the `trace_object` call to the underlying `Trace`,
    /// and flush as soon as `self.queue` is full.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        let result = self
            .trace
            .trace_object(self.worker, object, &mut self.queue);
        self.flush_if_full();
        result
    }
}

impl<'w, T: Trace> DefaultObjectTracer<'w, T> {
    fn new(worker: &'w mut GCWorker<T::VM>, trace: T, stage: WorkBucketStage) -> Self {
        Self {
            worker,
            trace,
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
        let work_packet = ProcessNodes::<T>::new(next_nodes, self.stage);
        self.worker.scheduler().work_buckets[self.stage].add(work_packet);
    }
}

/// This implementation of [`ObjectTracerContext`] creates the [`DefaultObjectTracer`] to expand the
/// transitive closure during a stop-the-world tracing GC or the final mark pause of a concurrent
/// GC.  It is used during object scanning as well as weak reference processing.
#[derive(Clone)]
pub(crate) struct DefaultObjectTracerContext<T: Trace> {
    stage: WorkBucketStage,
    phantom_data: PhantomData<T>,
}

impl<T: Trace> DefaultObjectTracerContext<T> {
    pub fn new(stage: WorkBucketStage) -> Self {
        Self {
            stage,
            phantom_data: PhantomData,
        }
    }
}

impl<T: Trace> ObjectTracerContext<T::VM> for DefaultObjectTracerContext<T> {
    type TracerType<'w> = DefaultObjectTracer<'w, T>;

    fn with_tracer<'w, R, F>(&self, worker: &'w mut GCWorker<T::VM>, func: F) -> R
    where
        F: FnOnce(&mut Self::TracerType<'w>) -> R,
    {
        let mmtk = worker.mmtk;

        // Create the callback tracer.
        let mut tracer = DefaultObjectTracer::new(worker, T::from_mmtk(mmtk), self.stage);

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
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, _mmtk: &'static MMTK<T::VM>) {
        trace!("VMProcessWeakRefs");

        let stage = WorkBucketStage::VMRefClosure;

        let need_to_repeat = {
            let tracer_factory = DefaultObjectTracerContext::<T>::new(stage);
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
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, _mmtk: &'static MMTK<T::VM>) {
        trace!("VMForwardWeakRefs");

        let stage = WorkBucketStage::VMRefForwarding;

        let tracer_factory = DefaultObjectTracerContext::<T>::new(stage);
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
