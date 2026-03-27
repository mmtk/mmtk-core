//! This module contains code useful for tracing,
//! i.e. visiting the reachable objects by traversing all or part of an object graph.

use std::marker::PhantomData;

use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::gc_work::{DefaultProcessSlots, DefaultScanObjects};
use crate::scheduler::{GCWork, GCWorker, WorkBucketStage, EDGES_WORK_BUFFER_SIZE};
use crate::util::{ObjectReference, VMThread, VMWorkerThread};
use crate::vm::{Scanning, VMBinding};
use crate::{Plan, MMTK};

pub trait TracePolicy: 'static + Send + Clone {
    type VM: VMBinding;
    type ProcessSlotsWorkType: GCWork<Self::VM>;
    type ScanObjectsWorkType: GCWork<Self::VM>;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self;

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference;

    fn post_scan_object(&mut self, object: ObjectReference);

    fn make_process_slots_work(
        &self,
        slots: Vec<<Self::VM as VMBinding>::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ProcessSlotsWorkType;

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType;

    fn may_move_objects() -> bool;
}

#[allow(dead_code)]
#[derive(Default)]
pub struct SFTTracePolicy<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> Clone for SFTTracePolicy<VM> {
    fn clone(&self) -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding> TracePolicy for SFTTracePolicy<VM> {
    type VM = VM;
    type ProcessSlotsWorkType = DefaultProcessSlots<Self>;
    type ScanObjectsWorkType = DefaultScanObjects<Self>;

    fn from_mmtk(_mmtk: &'static MMTK<Self::VM>) -> Self {
        Default::default()
    }

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        worker: &mut GCWorker<VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        use crate::policy::sft::GCWorkerMutRef;

        // Erase <VM> type parameter
        let worker = GCWorkerMutRef::new(worker);

        // Invoke trace object on sft
        let sft = unsafe { crate::mmtk::SFT_MAP.get_unchecked(object.to_raw_address()) };
        let mut tmp_queue = None;
        let result = sft.sft_trace_object(&mut tmp_queue, object, worker);
        if let Some(queued_object) = tmp_queue {
            queue.enqueue(queued_object);
        }
        result
    }

    fn post_scan_object(&mut self, _object: ObjectReference) {
        // Do nothing.  SFTTracePolicy is only suitable for plans that don't need post_scan_object.
    }

    fn make_process_slots_work(
        &self,
        slots: Vec<<Self::VM as VMBinding>::VMSlot>,
        roots: bool,
        _mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ProcessSlotsWorkType {
        DefaultProcessSlots::new(self.clone(), slots, roots, bucket)
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        _mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        DefaultScanObjects::new(self.clone(), nodes, bucket)
    }

    fn may_move_objects() -> bool {
        true
    }
}

pub struct PlanTracePolicy<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> {
    plan: &'static P,
}

impl<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> PlanTracePolicy<P, KIND> {
    pub(crate) fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> Clone for PlanTracePolicy<P, KIND> {
    fn clone(&self) -> Self {
        Self { plan: self.plan }
    }
}

impl<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> TracePolicy
    for PlanTracePolicy<P, KIND>
{
    type VM = P::VM;
    type ProcessSlotsWorkType = DefaultProcessSlots<Self>;
    type ScanObjectsWorkType = DefaultScanObjects<Self>;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<P>().unwrap();
        Self::new(plan)
    }

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        self.plan.trace_object::<Q, KIND>(queue, object, worker)
    }

    fn post_scan_object(&mut self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }

    fn make_process_slots_work(
        &self,
        slots: Vec<<Self::VM as VMBinding>::VMSlot>,
        roots: bool,
        _mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ProcessSlotsWorkType {
        DefaultProcessSlots::new(self.clone(), slots, roots, bucket)
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        _mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        DefaultScanObjects::new(self.clone(), nodes, bucket)
    }

    fn may_move_objects() -> bool {
        P::may_move_objects::<KIND>()
    }
}

#[derive(Default)]
pub struct UnsupportedTracePolicy<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> Clone for UnsupportedTracePolicy<VM> {
    fn clone(&self) -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding> TracePolicy for UnsupportedTracePolicy<VM> {
    type VM = VM;
    type ProcessSlotsWorkType = DefaultProcessSlots<Self>;
    type ScanObjectsWorkType = DefaultScanObjects<Self>;

    fn from_mmtk(_mmtk: &'static MMTK<Self::VM>) -> Self {
        unimplemented!()
    }

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        _worker: &mut GCWorker<VM>,
        _object: ObjectReference,
        _queue: &mut Q,
    ) -> ObjectReference {
        unimplemented!()
    }

    fn post_scan_object(&mut self, _object: ObjectReference) {
        unimplemented!()
    }

    fn make_process_slots_work(
        &self,
        _slots: Vec<<Self::VM as VMBinding>::VMSlot>,
        _roots: bool,
        _mmtk: &'static MMTK<Self::VM>,
        _bucket: WorkBucketStage,
    ) -> Self::ProcessSlotsWorkType {
        unimplemented!()
    }
    fn create_scan_work(
        &self,
        _nodes: Vec<ObjectReference>,
        _mmtk: &'static MMTK<Self::VM>,
        _bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        unimplemented!()
    }

    fn may_move_objects() -> bool {
        unimplemented!()
    }
}

/// This trait represents an object queue to enqueue objects during tracing.
pub trait ObjectQueue {
    /// Enqueue an object into the queue.
    fn enqueue(&mut self, object: ObjectReference);
}

impl<F: FnMut(ObjectReference)> ObjectQueue for F {
    fn enqueue(&mut self, object: ObjectReference) {
        self(object)
    }
}

impl ObjectQueue for Option<ObjectReference> {
    fn enqueue(&mut self, object: ObjectReference) {
        debug_assert!(self.is_none());
        *self = Some(object);
    }
}

/// A one-element object queue backed by an `Option<ObjectReference>`.  Used by SFT to decouple the
/// concrete [`ObjectQueue`] type from the dynamic dispatching.
pub type OptionObjectQueue = Option<ObjectReference>;

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

/// For iterating over the slots of an object.
// FIXME: This type iterates slots, but all of its current use cases only care about the values in the slots.
// And it currently only works if the object supports slot enqueuing (i.e. `Scanning::scan_object` is implemented).
// We may refactor the interface according to <https://github.com/mmtk/mmtk-core/issues/1375>
pub(crate) struct SlotIterator<VM: VMBinding> {
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> SlotIterator<VM> {
    /// Iterate over the slots of an object by applying a function to each slot.
    pub fn iterate_fields<F: FnMut(VM::VMSlot)>(object: ObjectReference, _tls: VMThread, mut f: F) {
        // FIXME: We should use tls from the arguments.
        // See https://github.com/mmtk/mmtk-core/issues/1375
        let fake_tls = VMWorkerThread(VMThread::UNINITIALIZED);
        if !<VM::VMScanning as Scanning<VM>>::support_slot_enqueuing(fake_tls, object) {
            panic!("SlotIterator::iterate_fields cannot be used on objects that don't support slot-enqueuing");
        }
        <VM::VMScanning as Scanning<VM>>::scan_object(fake_tls, object, &mut f);
    }
}
