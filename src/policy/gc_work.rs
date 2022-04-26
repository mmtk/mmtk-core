use crate::plan::Plan;
use crate::plan::TransitiveClosure;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;

use std::ops::{Deref, DerefMut};

/// Used to identify the trace if a policy has different kinds of traces. For example, defrag vs fast trace for Immix,
/// mark vs forward trace for mark compact.
pub(crate) type TraceKind = u8;

pub const DEFAULT_TRACE: u8 = u8::MAX;

/// A plan that uses `PolicyProcessEdges` needs to provide an implementation for this trait.
/// The plan needs to specify a target space (which needs to implement `SupportPolicyProcessEdges`).
/// For objects in the target space, `trace_object_with_tracekind()` is called to trace the object.
/// Otherwise, `fallback_trace()` is used.
/// For the sake of performance, the implementation of this trait should mark methods as `[inline(always)]`.
pub trait UsePolicyProcessEdges<VM: VMBinding>: Plan<VM = VM> + Send {
    type TargetPolicy: SupportPolicyProcessEdges<VM>;
    /// The copy semantics for objects in the space.
    const COPY: CopySemantics;

    /// Returns a reference to the target space.
    fn get_target_space(&self) -> &Self::TargetPolicy;

    /// How to trace objects if the object is not in the default space.
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;
}

/// A policy that allows using `PolicyProcessEdges` needs to provide an implementation for this trait.
/// For the sake of performance, the implementation of this trait should mark methods as `[inline(always)]`.
pub trait SupportPolicyProcessEdges<VM: VMBinding>: Space<VM> {
    /// Trace an object in the policy.
    fn trace_object_with_tracekind<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        copy: CopySemantics,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Create scan work for the policy. By default, we use [`ScanObjects`](crate::scheduler::gc_work::ScanObjects).
    /// If a policy has its own scan object work packet, they can override this method.
    #[inline(always)]
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>> {
        Box::new(crate::scheduler::gc_work::ScanObjects::<E>::new(
            nodes, false,
        ))
    }

    /// Does this trace move object?
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}

/// This provides an alternative to [`SFTProcessEdges`](crate::scheduler::gc_work::SFTProcessEdges). For policies that cannot
/// use `SFTProcessEdges`, they could try use this type. One major difference is that `PolicyProcessEdges` allows different
/// traces for a policy.
/// A plan that uses this needs to implement the `UsePolicyProcessEdges` trait, and the policy needs to implement `SupportPolicyProcessEdges`.
// pub struct PolicyProcessEdges<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> {
//     plan: &'static P,
//     base: ProcessEdgesBase<VM>,
// }

// impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> ProcessEdgesWork
//     for PolicyProcessEdges<VM, P, KIND>
// {
//     type VM = VM;

//     fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
//         let base = ProcessEdgesBase::new(edges, roots, mmtk);
//         let plan = base.plan().downcast_ref::<P>().unwrap();
//         Self { plan, base }
//     }

//     #[inline(always)]
//     fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Box<dyn GCWork<Self::VM>> {
//         self.plan.get_target_space().create_scan_work::<Self>(nodes)
//     }

//     /// Trace object if it is in the target space. Otherwise call fallback_trace().
//     #[inline(always)]
//     fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
//         if object.is_null() {
//             return object;
//         }
//         if self.plan.get_target_space().in_space(object) {
//             self.plan
//                 .get_target_space()
//                 .trace_object_with_tracekind::<Self, KIND>(self, object, P::COPY, self.worker())
//         } else {
//             self.plan
//                 .fallback_trace::<Self>(self, object, self.worker())
//         }
//     }

//     #[inline]
//     fn process_edge(&mut self, slot: Address) {
//         let object = unsafe { slot.load::<ObjectReference>() };
//         let new_object = self.trace_object(object);
//         if P::TargetPolicy::may_move_objects::<KIND>() {
//             unsafe { slot.store(new_object) };
//         }
//     }
// }

// // Impl Deref/DerefMut to ProcessEdgesBase for PolicyProcessEdges

// impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> Deref
//     for PolicyProcessEdges<VM, P, KIND>
// {
//     type Target = ProcessEdgesBase<VM>;
//     #[inline]
//     fn deref(&self) -> &Self::Target {
//         &self.base
//     }
// }

// impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> DerefMut
//     for PolicyProcessEdges<VM, P, KIND>
// {
//     #[inline]
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         &mut self.base
//     }
// }
use crate::plan::transitive_closure::PlanTraceObject;

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

    /// Trace object if it is in the target space. Otherwise call fallback_trace().
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

// Impl Deref/DerefMut to ProcessEdgesBase for PolicyProcessEdges

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
