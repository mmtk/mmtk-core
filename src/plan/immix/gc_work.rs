use super::global::Immix;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::copy::CopySemantics;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

// It would be better if we use an enum for this. However, we use this as
// a constant type parameter, and Rust only accepts integer and bool for
// constant type parameters for now. We need to wait until `adt_const_params` is
// stablized.
pub(in crate::plan) type TraceKind = u8;
pub(in crate::plan) const TRACE_KIND_FAST: TraceKind = 0;
pub(in crate::plan) const TRACE_KIND_DEFRAG: TraceKind = 1;

/// Object tracing for Immix.
/// Note that it is possible to use [`SFTProcessEdges`](mmtk/scheduler/gc_work/SFTProcessEdges) for immix.
/// We need to: 1. add a plan-specific method to create scan work packets, as most plans use `ScanObjects` while
/// immix uses `ScanObjectsAndMarkLines`, 2. use `ImmixSpace.trace_object()` which has an overhead of checking
/// which trace method to use (with ImmixProcessEdges, we can know which trace method to use by statically checking `TraceKind`).
pub(super) struct ImmixProcessEdges<VM: VMBinding, const KIND: TraceKind> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static Immix<VM>,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding, const KIND: TraceKind> ImmixProcessEdges<VM, KIND> {
    fn immix(&self) -> &'static Immix<VM> {
        self.plan
    }
}

impl<VM: VMBinding, const KIND: TraceKind> ProcessEdgesWork for ImmixProcessEdges<VM, KIND> {
    type VM = VM;

    const OVERWRITE_REFERENCE: bool = crate::policy::immix::DEFRAG;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<Immix<VM>>().unwrap();
        Self { plan, base }
    }

    #[cold]
    fn flush(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let scan_objects_work = crate::policy::immix::ScanObjectsAndMarkLines::<Self>::new(
            self.pop_nodes(),
            false,
            &self.immix().immix_space,
        );
        self.new_scan_work(scan_objects_work);
    }

    /// Trace  and evacuate objects.
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.immix().immix_space.in_space(object) {
            if KIND == TRACE_KIND_FAST {
                self.immix().immix_space.fast_trace_object(self, object)
            } else {
                self.immix().immix_space.trace_object(
                    self,
                    object,
                    CopySemantics::DefaultCopy,
                    self.worker(),
                )
            }
        } else {
            self.immix().common.trace_object::<Self>(self, object)
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

impl<VM: VMBinding, const KIND: TraceKind> Deref for ImmixProcessEdges<VM, KIND> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, const KIND: TraceKind> DerefMut for ImmixProcessEdges<VM, KIND> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub(super) struct ImmixGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ImmixGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = Immix<VM>;
    type ProcessEdgesWorkType = ImmixProcessEdges<VM, KIND>;
}
