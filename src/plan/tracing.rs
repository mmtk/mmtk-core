//! This module contains code useful for tracing,
//! i.e. visiting the reachable objects by traversing all or part of an object graph.

use crate::scheduler::gc_work::{EdgeOf, ProcessEdgesWork};
use crate::scheduler::{GCWorker, WorkBucketStage};
use crate::util::ObjectReference;
use crate::vm::{EdgeVisitor, VMBinding};

/// This trait represents an object queue to enqueue objects during tracing.
pub trait ObjectQueue {
    /// Enqueue an object into the queue.
    fn enqueue(&mut self, object: ObjectReference);
}

pub type VectorObjectQueue = VectorQueue<ObjectReference>;

/// An implementation of `ObjectQueue` using a `Vec`.
///
/// This can also be used as a buffer. For example, the mark stack or the write barrier mod-buffer.
pub struct VectorQueue<T> {
    /// Enqueued nodes.
    buffer: Vec<T>,
    /// Capacity of the queue.
    capacity: usize,
}

impl<T> VectorQueue<T> {
    /// The default capacity of the queue.
    const DEFAULT_CAPACITY: usize = 4096;

    /// Create an empty `VectorObjectQueue` with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Create an empty `VectorObjectQueue` with a given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::new(),
            capacity,
        }
    }

    /// Create an empty `VectorObjectQueue` with an optionally given capacity.
    pub fn with_capacity_opt(capacity_opt: Option<usize>) -> Self {
        if let Some(capacity) = capacity_opt {
            Self::with_capacity(capacity)
        } else {
            Self::new()
        }
    }

    /// Return `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Return the contents of the underlying vector.  It will empty the queue.
    pub fn take(&mut self) -> Vec<T> {
        std::mem::take(&mut self.buffer)
    }

    /// Consume this `VectorObjectQueue` and return its underlying vector.
    pub fn into_vec(self) -> Vec<T> {
        self.buffer
    }

    /// Check if the buffer size reaches the capacity.
    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.capacity
    }

    pub fn push(&mut self, v: T) {
        if self.buffer.is_empty() {
            self.buffer.reserve(self.capacity);
        }
        self.buffer.push(v);
    }
}

impl<T> Default for VectorQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectQueue for VectorQueue<ObjectReference> {
    fn enqueue(&mut self, v: ObjectReference) {
        self.push(v);
    }
}

/// A transitive closure visitor to collect all the edges of an object.
pub struct ObjectsClosure<'a, E: ProcessEdgesWork> {
    buffer: VectorQueue<EdgeOf<E>>,
    pub(crate) worker: &'a mut GCWorker<E::VM>,
    bucket: WorkBucketStage,
}

impl<'a, E: ProcessEdgesWork> ObjectsClosure<'a, E> {
    pub fn new(worker: &'a mut GCWorker<E::VM>, bucket: WorkBucketStage) -> Self {
        let buffer = VectorQueue::with_capacity_opt(E::VM::override_scan_objects_packet_size());
        Self {
            buffer,
            worker,
            bucket,
        }
    }

    fn flush(&mut self) {
        let buf = self.buffer.take();
        if !buf.is_empty() {
            self.worker.add_work(
                self.bucket,
                E::new(buf, false, self.worker.mmtk, self.bucket),
            );
        }
    }
}

impl<'a, E: ProcessEdgesWork> EdgeVisitor<EdgeOf<E>> for ObjectsClosure<'a, E> {
    fn visit_edge(&mut self, slot: EdgeOf<E>) {
        #[cfg(debug_assertions)]
        {
            use crate::vm::edge_shape::Edge;
            trace!(
                "(ObjectsClosure) Visit edge {:?} (pointing to {})",
                slot,
                slot.load()
            );
        }
        self.buffer.push(slot);
        if self.buffer.is_full() {
            self.flush();
        }
    }
}

impl<'a, E: ProcessEdgesWork> Drop for ObjectsClosure<'a, E> {
    fn drop(&mut self) {
        self.flush();
    }
}
