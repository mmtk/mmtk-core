//! This module contains code useful for tracing,
//! i.e. visiting the reachable objects by traversing all or part of an object graph.

use std::marker::PhantomData;

use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::{GCWorker, EDGES_WORK_BUFFER_SIZE};
use crate::util::{ObjectReference, VMThread, VMWorkerThread};
use crate::vm::{Scanning, VMBinding};
use crate::{Plan, MMTK};

/// This trait provides methods used during a trace.  The most important method is
/// [`Self::trace_object`] which provides the way to trace an object during the current trace.  Many
/// work packets depend on this trait to trace objects.
///
/// We need different implementations of this trait for the different behaviors in
///
/// -   different plans
/// -   different traces of the same plan (e.g. marking trace and forwarding trace; nursery GC and
///     mature GC; fast GC and defrag GC; etc.), and
/// -   pinning (transitive or not) roots and regular edges.
///
/// Therefore, each GC selects one [`GCWorkContext`], and each [`GCWorkContext`] selects two
/// [`Trace`] implementations for default edges and pinning edges, respectively.
///
/// A type of this trait shall be stateless and immutable.  It shall be cheap to instantiate from an
/// [`MMTK`] instance.
///
/// This trait requires the [`Clone`] trait, and cloned instances behave exactly the same.
///
/// [`GCWorkContext`]: crate::scheduler::work::GCWorkContext
pub trait Trace: 'static + Send + Clone {
    /// The VM binding type this type serves.
    type VM: VMBinding;

    /// Instantiate from an [`MMTK`] instance.
    ///
    /// Note that values of this trait are usually instantiated in work packets that use it. It
    /// should be reasonably cheap to instantiate.  Most types (such as [`PlanTrace`] and
    /// [`GenNurseryTrace`]) only need a reference to the plan which can be downcasted from
    /// [`MMTK::plan`].
    ///
    /// [`GenNurseryTrace`]: crate::plan::generational::gc_work::GenNurseryTrace
    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self;

    /// Trace the `object`.  More precisely, it visits an object graph *edge* that points to
    /// `object`.
    ///
    /// It may add `object` (or the new [`ObjectReference`] for `object` in a moving GC) into the
    /// `queue`, which means its children needs to be recursively traversed.
    ///
    /// In non-moving GC, the return value is always `object`.  In moving GC, the return value may
    /// be `object` or the new [`ObjectReference`] for the `object`.  If the return value is not
    /// `object`, the slot that represents the object graph edge needs to be updated to hold the
    /// return value instead.
    ///
    /// Its implementation generally needs to figure out which space an object resides in, and
    /// invoke the right "trace object" method of the space for the current trace.
    ///
    /// # Notes
    ///
    /// ## The enqueued value and the return value
    ///
    /// The return value may be different from the enqueued value.  For example, during the
    /// forwarding stage of MarkCompact, it returns the new object reference, but enqueues the old
    /// `object` because the object has not been moved, yet.
    ///
    /// ## The `queue` can be a callback instead of a collection
    ///
    /// `FnMut(ObjectReference)` implements the [`ObjectQueue`] trait.  This means you can use a
    /// lambda expression at the place of the `queue` argument.  For example, you can scan the
    /// object immediately instead of adding the object reference into a container.
    ///
    /// Example:
    ///
    /// ```rust
    /// trace.trace_object(worker, object, &mut |enqueued_object| {
    ///     // Process the enqueued_object here...
    /// });
    /// ```
    fn trace_object<Q: ObjectQueue>(
        &self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference;

    /// The post-scan hook to be call after scanning `object`.
    ///
    /// Each object is scanned by [`Scanning::scan_object`] or
    /// [`Scanning::scan_object_and_trace_edges`], and this function will be called after scanning
    /// an object as a hook to invoke possible policy-specific post-scan methods.  If `object` is in
    /// a space that needs such a hook, this method should call such hook of the space.  Otherwise,
    /// this method may do nothing.
    ///
    /// Currently, only [`ImmixSpace`] needs this hook to mark the line.
    ///
    /// [`ImmixSpace`]: crate::policy::immix::ImmixSpace
    fn post_scan_object(&self, object: ObjectReference);

    /// Return `true` if [`Self::trace_object`] may move any object.  If any space of the current
    /// plan may move objects during this trace, this method should return `true`.
    ///
    /// If it returns `false`, it means [`Self::trace_object`] is guaranteed not to move the
    /// `object`, and it is safe to elide updating a slot after tracing the reference in the slot.
    /// It is always correct to conservatively return `true`.
    ///
    /// Note that this method is called very frequently, so it must be efficient.
    fn may_move_objects() -> bool;
}

/// A shorthand for getting the slot type from a [`Trace`] instance.
pub type SlotOfTrace<T> = <<T as Trace>::VM as VMBinding>::VMSlot;

/// A [`Trace`] implementation that dispatches the `trace_object` method through
/// [`crate::policy::sft::SFT::sft_trace_object`] using the Space Function Table (SFT).
///
/// Because SFT methods cannot be general, `SFTTrace` cannot be used for plans that needs multiple
/// traces.  It is sufficient for simple plans such as `MarkSweep` and `SemiSpace`.
#[allow(dead_code)]
#[derive(Default)]
pub struct SFTTrace<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> Clone for SFTTrace<VM> {
    fn clone(&self) -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding> Trace for SFTTrace<VM> {
    type VM = VM;

    fn from_mmtk(_mmtk: &'static MMTK<Self::VM>) -> Self {
        Default::default()
    }

    fn trace_object<Q: ObjectQueue>(
        &self,
        worker: &mut GCWorker<VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        use crate::policy::sft::GCWorkerMutRef;

        // Erase <VM> type parameter
        let worker = GCWorkerMutRef::new(worker);

        // Invoke trace object on sft
        let sft = unsafe { crate::mmtk::SFT_MAP.get_unchecked(object.to_raw_address()) };

        // Because `sft.sft_trace_object` cannot have generic parameters, we can't pass `queue`
        // directly to it.  Instead we let `sft_trace_object` enqueue to this `tmp_queue` and
        // forward the enqueued object to `queue`.
        let mut tmp_queue = None;
        let result = sft.sft_trace_object(&mut tmp_queue, object, worker);
        if let Some(queued_object) = tmp_queue {
            queue.enqueue(queued_object);
        }
        result
    }

    fn post_scan_object(&self, _object: ObjectReference) {
        // Do nothing.  SFTTrace is only suitable for plans that don't need post_scan_object.
    }

    fn may_move_objects() -> bool {
        // We conservatively assume it may move objects.
        true
    }
}

/// A [`Trace`] implementation that dispatches the `trace_object` method through
/// [`PlanTraceObject::trace_object`].  It is applicable to all plans that implement
/// [`PlanTraceObject`].
///
/// Plans usually don't implement `PlanTraceObject` directly, but use the
/// `#[derive(PlanTraceObject)]` macro. See [`PlanTraceObject`] for more details.
pub struct PlanTrace<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> {
    plan: &'static P,
}

impl<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> Clone for PlanTrace<P, KIND> {
    fn clone(&self) -> Self {
        Self { plan: self.plan }
    }
}

impl<P: Plan + PlanTraceObject<P::VM>, const KIND: TraceKind> Trace for PlanTrace<P, KIND> {
    type VM = P::VM;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<P>().unwrap();
        Self { plan }
    }

    fn trace_object<Q: ObjectQueue>(
        &self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        self.plan.trace_object::<Q, KIND>(queue, object, worker)
    }

    fn post_scan_object(&self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }

    fn may_move_objects() -> bool {
        P::may_move_objects::<KIND>()
    }
}

/// A placeholder for unsupported traces.  For example, it can be used for
/// [`crate::scheduler::GCWorkContext::PinningTrace`] for plans that don't support object pinning.
#[derive(Default)]
pub struct UnsupportedTrace<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> Clone for UnsupportedTrace<VM> {
    fn clone(&self) -> Self {
        Self {
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding> Trace for UnsupportedTrace<VM> {
    type VM = VM;

    fn from_mmtk(_mmtk: &'static MMTK<Self::VM>) -> Self {
        panic!("UnsupportedTrace cannot be constructed.")
    }

    fn trace_object<Q: ObjectQueue>(
        &self,
        _worker: &mut GCWorker<VM>,
        _object: ObjectReference,
        _queue: &mut Q,
    ) -> ObjectReference {
        panic!("UnsupportedTrace::trace_object must not be called.")
    }

    fn post_scan_object(&self, _object: ObjectReference) {
        panic!("UnsupportedTrace::post_scan_object must not be called.")
    }

    fn may_move_objects() -> bool {
        panic!("UnsupportedTrace::may_move_objects must not be called.")
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
