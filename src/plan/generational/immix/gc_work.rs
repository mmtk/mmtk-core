use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

use crate::plan::immix::gc_work::{TraceKind, TRACE_KIND_FAST};

/// ProcessEdges for a full heap GC for generational immix. The const type parameter
/// defines whether there is copying in the GC.
/// Note that even with TraceKind::Fast, there is no defragmentation, we are still
/// copying from nursery to immix space. So we always need to write new object
/// references in process_edge() (i.e. we do not need to overwrite the default implementation
/// of process_edge() as the immix plan does).
pub(super) struct GenImmixMatureProcessEdges<VM: VMBinding, const KIND: TraceKind> {
    plan: &'static GenImmix<VM>,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding, const KIND: TraceKind> ProcessEdgesWork
    for GenImmixMatureProcessEdges<VM, KIND>
{
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<GenImmix<VM>>().unwrap();
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
            &self.plan.immix,
        );
        self.new_scan_work(scan_objects_work);
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }

        if self.plan.immix.in_space(object) {
            if KIND == TRACE_KIND_FAST {
                return self.plan.immix.fast_trace_object(self, object);
            } else {
                return self.plan.immix.trace_object(
                    self,
                    object,
                    crate::util::copy::CopySemantics::Mature,
                    self.worker(),
                );
            }
        }

        self.plan
            .gen
            .trace_object_full_heap::<Self>(self, object, self.worker())
    }
}

impl<VM: VMBinding, const KIND: TraceKind> Deref for GenImmixMatureProcessEdges<VM, KIND> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, const KIND: TraceKind> DerefMut for GenImmixMatureProcessEdges<VM, KIND> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = GenImmixMatureProcessEdges<VM, KIND>;
}
