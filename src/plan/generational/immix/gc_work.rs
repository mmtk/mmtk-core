use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryTrace;
use crate::plan::tracing::PlanTrace;
use crate::plan::tracing::UnsupportedTrace;
use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultTrace = GenNurseryTrace<VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningTrace = UnsupportedTrace<VM>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type DefaultTrace = PlanTrace<GenImmix<VM>, KIND>;
    type PinningTrace = UnsupportedTrace<VM>;
}
