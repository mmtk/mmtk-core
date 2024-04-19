use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::gc_work::PlanProcessEdges;
use crate::scheduler::gc_work::UnsupportedProcessEdges;
use crate::vm::VMBinding;

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultProcessEdges = GenNurseryProcessEdges<VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultProcessEdges = PlanProcessEdges<VM, GenImmix<VM>, KIND>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}
