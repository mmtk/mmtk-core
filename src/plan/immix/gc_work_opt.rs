use super::global::Immix;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::copy::CopySemantics;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub(in crate::plan) enum TraceKind {
    Fast,
    Defrag,
}

pub(super) struct ImmixProcessEdges<VM: VMBinding, const KIND: TraceKind> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static Immix<VM>,
    mmtk_process_edges: MMTkProcessEdges<VM>,
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
        let plan = mmtk.plan.downcast_ref::<Immix<VM>>().unwrap();
        Self { plan, mmtk_process_edges: MMTkProcessEdges::new(edges, roots, mmtk) }
    }

    #[cold]
    fn flush(&mut self) {
        self.mmtk_process_edges.flush()
    }

    /// Trace  and evacuate objects.
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.immix().immix_space.in_space(object) {
            if KIND == TraceKind::Fast {
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
            self.mmtk_process_edges.trace_object(object)
        }
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        if KIND == TraceKind::Defrag && Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }
}

impl<VM: VMBinding, const KIND: TraceKind> Deref for ImmixProcessEdges<VM, KIND> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.mmtk_process_edges.base
    }
}

impl<VM: VMBinding, const KIND: TraceKind> DerefMut for ImmixProcessEdges<VM, KIND> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.mmtk_process_edges.base
    }
}

use crate::scheduler::gc_work::MMTkProcessEdges;
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
