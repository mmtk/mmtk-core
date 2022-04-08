use crate::plan::Plan;
use crate::plan::TransitiveClosure;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;

use std::ops::{Deref, DerefMut};

/// Used to identify the trace if a policy has different kinds of traces. For example, defrag vs fast trace for Immix,
/// mark vs forward trace for mark compact.
pub(crate) type TraceKind = u8;

/// A plan that uses `PolicyProcessEdges` needs to provide an implementation for this trait.
/// The plan needs to specify a target space. For objects in the target space, `target_trace()` is used to trace the object.
/// Otherwise, `fallback_trace()` is used.
/// For the sake of performance, the implementation of this trait should mark methods as `[inline(always)]`.
pub trait UsePolicyProcessEdges<VM: VMBinding>: Plan<VM = VM> + Send {
    type DefaultSpaceType: Space<VM>;

    /// Returns a reference to the default space.
    fn get_target_space(&self) -> &Self::DefaultSpaceType;

    /// How to trace object in the default space
    fn target_trace<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Does this trace move object?
    fn may_move_objects<const KIND: TraceKind>() -> bool;

    /// How to trace objects if the object is not in the default space.
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Create a scan work from the nodes. By default, the `ScanObjects` work packet is used. If a policy
    /// uses their own scan work packet, they should override this method.
    #[inline(always)]
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>> {
        Box::new(crate::scheduler::gc_work::ScanObjects::<E>::new(
            nodes, false,
        ))
    }
}

/// This provides an alternative to [`SFTProcessEdges`](crate::scheduler::gc_work::SFTProcessEdges). For policies that cannot
/// use `SFTProcessEdges`, they could try use this type. One major difference is that `PolicyProcessEdges` allows different
/// traces for a policy.
/// A plan that uses this needs to implement the `UsePolicyProcessEdges` trait, and should choose the policy that has multiple
/// traces as the 'target'. See more details for [`UsePolicyProcessEdges`](crate::policy::gc_work::UsePolicyProcessEdges).
pub struct PolicyProcessEdges<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> ProcessEdgesWork
    for PolicyProcessEdges<VM, P, KIND>
{
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        Self { plan, base }
    }

    #[cold]
    fn flush(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let scan_objects_work = self.plan.create_scan_work::<Self>(self.pop_nodes());
        self.new_scan_work(scan_objects_work);
    }

    /// Trace object if it is in the target space. Otherwise call fallback_trace().
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan.get_target_space().in_space(object) {
            self.plan
                .target_trace::<Self, KIND>(self, object, self.worker())
        } else {
            self.plan
                .fallback_trace::<Self>(self, object, self.worker())
        }
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

// Impl Deref/DerefMut to ProcessEdgesBase for PolicyProcessEdges

impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> Deref
    for PolicyProcessEdges<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> DerefMut
    for PolicyProcessEdges<VM, P, KIND>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
