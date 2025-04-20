//! This module contains code useful for tracing,
//! i.e. visiting the reachable objects by traversing all or part of an object graph.

use std::marker::PhantomData;

use crate::scheduler::gc_work::{ProcessEdgesWork, SlotOf};
use crate::scheduler::{GCWorker, WorkBucketStage};
use crate::util::Address;
use crate::util::{ObjectReference, VMThread, VMWorkerThread};
use crate::vm::SlotVisitor;
use crate::vm::{Scanning, VMBinding};

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

impl<T: Clone> VectorQueue<T> {
    pub fn clone_buffer(&self) -> Vec<T> {
        self.buffer.clone()
    }
}

impl<T> VectorQueue<T> {
    /// Reserve a capacity of this on first enqueue to avoid frequent resizing.
    const CAPACITY: usize = crate::args::BUFFER_SIZE;

    /// Create an empty `VectorObjectQueue`.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Return `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
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

    pub fn swap(&mut self, new_buffer: &mut Vec<T>) {
        std::mem::swap(&mut self.buffer, new_buffer)
    }

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
    should_discover_references: bool,
    should_claim_and_scan_clds: bool,
    bucket: WorkBucketStage,
}

impl<'a, E: ProcessEdgesWork> ObjectsClosure<'a, E> {
    /// Create an [`ObjectsClosure`].
    ///
    /// Arguments:
    /// * `worker`: the current worker. The objects closure should not leave the context of this worker.
    /// * `bucket`: new work generated will be push ed to the bucket.
    pub fn new(
        worker: &'a mut GCWorker<E::VM>,
        should_discover_references: bool,
        should_claim_and_scan_clds: bool,
        bucket: WorkBucketStage,
    ) -> Self {
        Self {
            buffer: VectorQueue::new(),
            worker,
            should_discover_references,
            should_claim_and_scan_clds,
            bucket,
        }
    }

    fn flush(&mut self) {
        let buf = VectorQueue::take(&mut self.buffer);
        if !buf.is_empty() {
            self.worker.add_work(
                self.bucket,
                E::new(buf, false, self.worker.mmtk, self.bucket),
            );
        }
    }
}

impl<E: ProcessEdgesWork> SlotVisitor<SlotOf<E>> for ObjectsClosure<'_, E> {
    fn should_discover_references(&self) -> bool {
        self.should_discover_references
    }
    fn should_claim_clds(&self) -> bool {
        self.should_claim_and_scan_clds
    }
    fn should_follow_clds(&self) -> bool {
        self.should_claim_and_scan_clds
    }
    fn visit_slot(&mut self, slot: SlotOf<E>, _out_of_heap: bool) {
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
        if self.buffer.is_empty() {
            return;
        }
        self.flush();
    }
}

struct SlotIteratorImpl<VM: VMBinding, F: FnMut(VM::VMSlot, bool)> {
    f: F,
    should_discover_references: bool,
    should_claim_clds: bool,
    should_follow_clds: bool,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding, F: FnMut(VM::VMSlot, bool)> SlotVisitor<VM::VMSlot>
    for SlotIteratorImpl<VM, F>
{
    fn should_discover_references(&self) -> bool {
        self.should_discover_references
    }
    fn should_claim_clds(&self) -> bool {
        self.should_claim_clds
    }
    fn should_follow_clds(&self) -> bool {
        self.should_follow_clds
    }
    fn visit_slot(&mut self, slot: VM::VMSlot, out_of_heap: bool) {
        (self.f)(slot, out_of_heap);
    }
}

pub struct SlotIterator<VM: VMBinding> {
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> SlotIterator<VM> {
    pub fn iterate(
        o: ObjectReference,
        should_discover_references: bool,
        should_claim_clds: bool,
        should_follow_clds: bool,
        f: impl FnMut(VM::VMSlot, bool),
        klass: Option<Address>,
    ) {
        let mut x = SlotIteratorImpl::<VM, _> {
            f,
            should_discover_references,
            should_claim_clds,
            should_follow_clds,
            _p: PhantomData,
        };
        if let Some(klass) = klass {
            <VM::VMScanning as Scanning<VM>>::scan_object_with_klass(
                VMWorkerThread(VMThread::UNINITIALIZED),
                o,
                &mut x,
                klass,
            );
        } else {
            <VM::VMScanning as Scanning<VM>>::scan_object(
                VMWorkerThread(VMThread::UNINITIALIZED),
                o,
                &mut x,
            );
        }
    }
}
