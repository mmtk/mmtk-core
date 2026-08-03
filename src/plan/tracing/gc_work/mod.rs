pub(crate) mod closure;
pub(crate) mod root;
pub(crate) mod weakref;

use std::marker::PhantomData;

use crate::{
    plan::{
        tracing::{gc_work::closure::ProcessNodes, Trace},
        VectorObjectQueue,
    },
    scheduler::{GCWorker, WorkBucketStage},
    util::ObjectReference,
    vm::{ObjectTracer, ObjectTracerContext},
};

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
