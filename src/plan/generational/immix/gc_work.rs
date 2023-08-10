use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::TRACE_KIND_IMMOVABLE;
use crate::scheduler::gc_work::PlanProcessEdges;
use crate::vm::VMBinding;

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM, Self::PlanType>;
    type ImmovableProcessEdges = GenNurseryProcessEdges<VM, Self::PlanType>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<VM, GenImmix<VM>, KIND>;
    type ImmovableProcessEdges = PlanProcessEdges<VM, GenImmix<VM>, TRACE_KIND_IMMOVABLE>;
}
