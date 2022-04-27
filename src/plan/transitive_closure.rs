//! The fundamental mechanism for performing a transitive closure over an object graph.

use std::mem;

use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::{GCWorker, WorkBucketStage};
use crate::util::{Address, ObjectReference};
use crate::MMTK;

/// This trait is the fundamental mechanism for performing a
/// transitive closure over an object graph.
pub trait TransitiveClosure {
    // The signature of this function changes during the port
    // because the argument `ObjectReference source` is never used in the original version
    // See issue #5
    fn process_edge(&mut self, slot: Address);
    fn process_node(&mut self, object: ObjectReference);
}

impl<T: ProcessEdgesWork> TransitiveClosure for T {
    fn process_edge(&mut self, _slot: Address) {
        unreachable!();
    }
    #[inline]
    fn process_node(&mut self, object: ObjectReference) {
        ProcessEdgesWork::process_node(self, object);
    }
}

/// A transitive closure visitor to collect all the edges of an object.
pub struct ObjectsClosure<'a, E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    buffer: Vec<Address>,
    worker: &'a mut GCWorker<E::VM>,
}

impl<'a, E: ProcessEdgesWork> ObjectsClosure<'a, E> {
    pub fn new(
        mmtk: &'static MMTK<E::VM>,
        buffer: Vec<Address>,
        worker: &'a mut GCWorker<E::VM>,
    ) -> Self {
        Self {
            mmtk,
            buffer,
            worker,
        }
    }
}

impl<'a, E: ProcessEdgesWork> TransitiveClosure for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn process_edge(&mut self, slot: Address) {
        if self.buffer.is_empty() {
            self.buffer.reserve(E::CAPACITY);
        }
        self.buffer.push(slot);
        if self.buffer.len() >= E::CAPACITY {
            let mut new_edges = Vec::new();
            mem::swap(&mut new_edges, &mut self.buffer);
            self.worker.add_work(
                WorkBucketStage::Closure,
                E::new(new_edges, false, self.mmtk),
            );
        }
    }
    fn process_node(&mut self, _object: ObjectReference) {
        unreachable!()
    }
}

impl<'a, E: ProcessEdgesWork> Drop for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn drop(&mut self) {
        let mut new_edges = Vec::new();
        mem::swap(&mut new_edges, &mut self.buffer);
        self.worker.add_work(
            WorkBucketStage::Closure,
            E::new(new_edges, false, self.mmtk),
        );
    }
}

use crate::policy::gc_work::TraceKind;
use crate::scheduler::GCWork;
use crate::vm::VMBinding;

/// A plan that uses `PlanProcessEdges` needs to provide an implementation for this trait.
/// Generally a plan does not need to manually implement this trait. Instead, we provide
/// a procedural macro that helps generate an implementation. Please check `macros/trace_object`.
///
/// A plan could also manually implement this trait. For the sake of performance, the implementation
/// of this trait should mark methods as `[inline(always)]`.
pub trait PlanTraceObject<VM: VMBinding> {
    /// Trace objects in the plan. Generally one needs to figure out
    /// which space an object resides in, and invokes the corresponding policy
    /// trace object method.
    fn trace_object<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Create a scan objects work packet for the plan. Usually [`ScanObjects`](scheduler/gc_work/ScanObjects)
    /// is used. If a plan or any policy in the plan uses a specific scan work packet, the work packet is required
    /// to handle objects that is in any space in the plan.
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>>;

    /// Whether objects in this plan may move. If any of the spaces used by the plan may move objects, this should
    /// return true.
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}

use crate::plan::Plan;
use crate::scheduler::gc_work::ProcessEdgesBase;
use std::ops::{Deref, DerefMut};

/// This provides an implementation of [`ProcessEdgesWork`](scheduler/gc_work/ProcessEdgesWork). A plan that implements
/// `PlanTraceObject` can use this work packet for tracing objects.
pub struct PlanProcessEdges<
    VM: VMBinding,
    P: 'static + Plan<VM = VM> + PlanTraceObject<VM> + Sync,
    const KIND: TraceKind,
> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
}

impl<
        VM: VMBinding,
        P: 'static + PlanTraceObject<VM> + Plan<VM = VM> + Sync,
        const KIND: TraceKind,
    > ProcessEdgesWork for PlanProcessEdges<VM, P, KIND>
{
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        Self { plan, base }
    }

    #[inline(always)]
    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Box<dyn GCWork<Self::VM>> {
        self.plan.create_scan_work::<Self>(nodes)
    }

    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        self.plan
            .trace_object::<Self, KIND>(self, object, self.worker())
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if P::may_move_objects::<KIND>() {
            unsafe { slot.store(new_object) };
        }
    }
}

// Impl Deref/DerefMut to ProcessEdgesBase for PlanProcessEdges
impl<
        VM: VMBinding,
        P: 'static + PlanTraceObject<VM> + Plan<VM = VM> + Sync,
        const KIND: TraceKind,
    > Deref for PlanProcessEdges<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<
        VM: VMBinding,
        P: 'static + PlanTraceObject<VM> + Plan<VM = VM> + Sync,
        const KIND: TraceKind,
    > DerefMut for PlanProcessEdges<VM, P, KIND>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
