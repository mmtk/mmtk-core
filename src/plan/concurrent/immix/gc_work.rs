use crate::plan::concurrent::immix::global::ConcurrentImmix;
use crate::plan::tracing::{EdgeTracer, PlanEdgeTracer};
use crate::policy::gc_work::{TraceKind, TRACE_KIND_TRANSITIVE_PIN};
use crate::vm::VMBinding;

pub(super) struct ConcurrentImmixSTWGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ConcurrentImmixSTWGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = ConcurrentImmix<VM>;
    type DefaultEdgeTracer = PlanEdgeTracer<ConcurrentImmix<VM>, KIND>;
    type PinningEdgeTracer = PlanEdgeTracer<ConcurrentImmix<VM>, TRACE_KIND_TRANSITIVE_PIN>;
}
pub(super) struct ConcurrentImmixGCWorkContext<E: EdgeTracer>(std::marker::PhantomData<E>);

impl<E: EdgeTracer> crate::scheduler::GCWorkContext for ConcurrentImmixGCWorkContext<E> {
    type VM = E::VM;
    type PlanType = ConcurrentImmix<E::VM>;
    type DefaultEdgeTracer = E;
    type PinningEdgeTracer = E;
}
