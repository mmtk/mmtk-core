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

use super::ImmixSpace;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

// It would be better if we use an enum for this. However, we use this as
// a constant type parameter, and Rust only accepts integer and bool for
// constant type parameters for now. We need to wait until `adt_const_params` is
// stablized.
pub(crate) type TraceKind = u8;
pub(crate) const TRACE_KIND_FAST: TraceKind = 0;
pub(crate) const TRACE_KIND_DEFRAG: TraceKind = 1;

/// An Immix plan can use `ImmixProcessEdgesWork`. This assumes there is only one immix space in the plan.
/// For the sake of performance, the implementation of these methods should be marked as `inline(always)`.
pub trait ImmixPlan<VM: VMBinding>: Plan<VM = VM> + Send {
    /// Returns a reference to the immix space.
    fn get_immix_space(&'static self) -> &'static ImmixSpace<VM>;
    /// Returns the copy semantic for the immix space.
    fn get_immix_copy_semantics() -> CopySemantics;
    /// How to trace objects if the object is not in the immix space.
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;
}

/// Object tracing for Immix.
/// Note that it is possible to use [`SFTProcessEdges`](mmtk/scheduler/gc_work/SFTProcessEdges) for immix.
/// We need to: 1. add a plan-specific method to create scan work packets, as most plans use `ScanObjects` while
/// immix uses `ScanObjectsAndMarkLines`, 2. use `ImmixSpace.trace_object()` which has an overhead of checking
/// which trace method to use (with ImmixProcessEdges, we can know which trace method to use by statically checking `TraceKind`).
pub struct ImmixProcessEdgesWork<VM: VMBinding, P: ImmixPlan<VM>, const KIND: TraceKind> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
    p: PhantomData<P>,
}

impl<VM: VMBinding, P: ImmixPlan<VM>, const KIND: TraceKind> ProcessEdgesWork
    for ImmixProcessEdgesWork<VM, P, KIND>
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
        let scan_objects_work = crate::policy::immix::ScanObjectsAndMarkLines::<Self>::new(
            self.pop_nodes(),
            false,
            self.plan.get_immix_space(),
        );
        self.new_scan_work(scan_objects_work);
    }

    /// Trace and evacuate objects.
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan.get_immix_space().in_space(object) {
            if KIND == TRACE_KIND_FAST {
                self.plan.get_immix_space().fast_trace_object(self, object)
            } else {
                self.plan.get_immix_space().trace_object(
                    self,
                    object,
                    P::get_immix_copy_semantics(),
                    self.worker(),
                )
            }
        } else {
            self.plan
                .fallback_trace::<Self>(self, object, self.worker())
        }
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if KIND == TRACE_KIND_DEFRAG && Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }
}

impl<VM: VMBinding, P: ImmixPlan<VM>, const KIND: TraceKind> Deref
    for ImmixProcessEdgesWork<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: ImmixPlan<VM>, const KIND: TraceKind> DerefMut
    for ImmixProcessEdgesWork<VM, P, KIND>
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
