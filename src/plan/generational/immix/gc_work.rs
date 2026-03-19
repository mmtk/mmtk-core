use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryTracePolicy;
use crate::plan::tracing::PlanTracePolicy;
use crate::plan::tracing::UnsupportedTracePolicy;
use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultTracePolicy = GenNurseryTracePolicy<VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningTracePolicy = UnsupportedTracePolicy<VM>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultTracePolicy = PlanTracePolicy<GenImmix<VM>, KIND>;
    type PinningTracePolicy = UnsupportedTracePolicy<VM>;
}
