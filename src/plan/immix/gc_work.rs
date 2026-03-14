use super::global::Immix;
use crate::plan::tracing::PlanEdgeTracer;
use crate::policy::gc_work::TraceKind;
use crate::policy::gc_work::TRACE_KIND_TRANSITIVE_PIN;
use crate::vm::VMBinding;

pub(super) struct ImmixGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ImmixGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = Immix<VM>;
    type DefaultEdgeTracer = PlanEdgeTracer<Immix<VM>, KIND>;
    type PinningEdgeTracer = PlanEdgeTracer<Immix<VM>, TRACE_KIND_TRANSITIVE_PIN>;
}
