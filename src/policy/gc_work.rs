use crate::plan::Plan;
use crate::plan::TransitiveClosure;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::scheduler::GCWork;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub(crate) type TraceKind = u8;

/// An Immix plan can use `ImmixProcessEdgesWork`. This assumes there is only one immix space in the plan.
/// For the sake of performance, the implementation of these methods should be marked as `inline(always)`.
pub trait UsePolicyProcessEdges<VM: VMBinding>: Plan<VM = VM> + Send {
    type SpaceType: Space<VM>;

    /// Returns a reference to the immix space.
    fn get_target_space(&'static self) -> &'static Self::SpaceType;
    /// Returns the copy semantic for the immix space.
    fn get_target_copy_semantics<const KIND: TraceKind>() -> CopySemantics;
    /// How to trace object in target space
    fn target_trace<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;
    fn overwrite_reference<const KIND: TraceKind>() -> bool;
    /// How to trace objects if the object is not in the immix space.
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(&'static self, nodes: Vec<ObjectReference>) -> Box<dyn GCWork<VM>> {
        Box::new(crate::scheduler::gc_work::ScanObjects::<E>::new(nodes, false))
    }
}

pub struct PolicyProcessEdges<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
    p: PhantomData<P>,
}

impl<VM: VMBinding, P: UsePolicyProcessEdges<VM>, const KIND: TraceKind> ProcessEdgesWork
    for PolicyProcessEdges<VM, P, KIND>
{
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        Self {
            plan,
            base,
            p: PhantomData,
        }
    }

    #[cold]
    fn flush(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let scan_objects_work = self.plan.create_scan_work::<Self>(self.pop_nodes());
        self.new_scan_work(scan_objects_work);
    }

    /// Trace and evacuate objects.
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan.get_target_space().in_space(object) {
            self.plan.target_trace::<Self, KIND>(self, object, self.worker())
        } else {
            self.plan
                .fallback_trace::<Self>(self, object, self.worker())
        }
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if P::overwrite_reference::<KIND>() && Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }
}

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
