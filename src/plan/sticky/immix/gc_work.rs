use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::TRACE_KIND_TRANSITIVE_PIN;
use crate::scheduler::gc_work::PlanProcessEdges;
use crate::{plan::generational::gc_work::GenNurseryProcessEdges, vm::VMBinding};

use super::global::StickyImmix;

pub struct StickyImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for StickyImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = StickyImmix<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM, Self::PlanType>;
    type TPProcessEdges = GenNurseryProcessEdges<VM, Self::PlanType>;
}

pub struct StickyImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for StickyImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = StickyImmix<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<VM, Self::PlanType, KIND>;
    type TPProcessEdges = PlanProcessEdges<VM, Self::PlanType, TRACE_KIND_TRANSITIVE_PIN>;
}
