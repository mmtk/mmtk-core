//! This module contains code useful for tracing,
//! i.e. visiting the reachable objects by traversing all or part of an object graph.

use std::marker::PhantomData;

use crate::scheduler::gc_work::{ProcessEdgesWork, SlotOf};
use crate::scheduler::{GCWorker, WorkBucketStage, EDGES_WORK_BUFFER_SIZE};
use crate::util::{ObjectReference, VMThread, VMWorkerThread};
use crate::vm::{Scanning, SlotVisitor, VMBinding};

/// This trait represents an object queue to enqueue objects during tracing.
pub trait ObjectQueue {
    /// Enqueue an object into the queue.
    fn enqueue(&mut self, object: ObjectReference);
}

/// A vector queue for object references.
pub type VectorObjectQueue = VectorQueue<ObjectReference>;

/// An implementation of `ObjectQueue` using a `Vec`.
///
/// This can also be used as a buffer. For example, the mark stack or the write barrier mod-buffer.
pub struct VectorQueue<T> {
    /// Enqueued nodes.
    buffer: Vec<T>,
}

impl<T> VectorQueue<T> {
    /// Reserve a capacity of this on first enqueue to avoid frequent resizing.
    const CAPACITY: usize = EDGES_WORK_BUFFER_SIZE;

    /// Create an empty `VectorObjectQueue`.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
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

    /// Check if the buffer size reaches `CAPACITY`.
    pub fn is_full(&self) -> bool {
        self.buffer.len() >= Self::CAPACITY
    }

    /// Push an element to the queue. If the queue is empty, it will reserve
    /// space to hold the number of elements defined by the capacity.
    /// The user of this method needs to make sure the queue length does
    /// not exceed the capacity to avoid allocating more space
    /// (this method will not check the length against the capacity).
    pub fn push(&mut self, v: T) {
        if self.buffer.is_empty() {
            self.buffer.reserve(Self::CAPACITY);
        }
        self.buffer.push(v);
    }

    /// Return the len of the queue
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Empty the queue
    pub fn clear(&mut self) {
        self.buffer.clear()
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

/// A transitive closure visitor to collect the slots from objects.
/// It maintains a buffer for the slots, and flushes slots to a new work packet
/// if the buffer is full or if the type gets dropped.
pub struct ObjectsClosure<'a, E: ProcessEdgesWork> {
    buffer: VectorQueue<SlotOf<E>>,
    pub(crate) worker: &'a mut GCWorker<E::VM>,
    bucket: WorkBucketStage,
}

impl<'a, E: ProcessEdgesWork> ObjectsClosure<'a, E> {
    /// Create an [`ObjectsClosure`].
    ///
    /// Arguments:
    /// * `worker`: the current worker. The objects closure should not leave the context of this worker.
    /// * `bucket`: new work generated will be push ed to the bucket.
    pub fn new(worker: &'a mut GCWorker<E::VM>, bucket: WorkBucketStage) -> Self {
        Self {
            buffer: VectorQueue::new(),
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

impl<E: ProcessEdgesWork> SlotVisitor<SlotOf<E>> for ObjectsClosure<'_, E> {
    fn visit_slot(&mut self, slot: SlotOf<E>) {
        #[cfg(debug_assertions)]
        {
            use crate::vm::slot::Slot;
            trace!(
                "(ObjectsClosure) Visit slot {:?} (pointing to {:?})",
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

impl<E: ProcessEdgesWork> Drop for ObjectsClosure<'_, E> {
    fn drop(&mut self) {
        self.flush();
    }
}

struct SlotIteratorImpl<VM: VMBinding, F: FnMut(VM::VMSlot)> {
    f: F,
    // should_discover_references: bool,
    // should_claim_clds: bool,
    // should_follow_clds: bool,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding, F: FnMut(VM::VMSlot)> SlotVisitor<VM::VMSlot> for SlotIteratorImpl<VM, F> {
    fn visit_slot(&mut self, slot: VM::VMSlot) {
        (self.f)(slot);
    }
}

pub struct SlotIterator<VM: VMBinding> {
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> SlotIterator<VM> {
    pub fn iterate(
        o: ObjectReference,
        // should_discover_references: bool,
        // should_claim_clds: bool,
        // should_follow_clds: bool,
        f: impl FnMut(VM::VMSlot),
        // klass: Option<Address>,
    ) {
        let mut x = SlotIteratorImpl::<VM, _> {
            f,
            // should_discover_references,
            // should_claim_clds,
            // should_follow_clds,
            _p: PhantomData,
        };
        // if let Some(klass) = klass {
        //     <VM::VMScanning as Scanning<VM>>::scan_object_with_klass(
        //         VMWorkerThread(VMThread::UNINITIALIZED),
        //         o,
        //         &mut x,
        //         klass,
        //     );
        // } else {
        //     <VM::VMScanning as Scanning<VM>>::scan_object(
        //         VMWorkerThread(VMThread::UNINITIALIZED),
        //         o,
        //         &mut x,
        //     );
        // }
        <VM::VMScanning as Scanning<VM>>::scan_object(
            VMWorkerThread(VMThread::UNINITIALIZED),
            o,
            &mut x,
        );
    }
}
