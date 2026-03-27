use crate::plan::concurrent::concurrent_marking_work::ConcurrentMarkingRootsWorkFactory;
use crate::plan::concurrent::immix::global::ConcurrentImmix;
use crate::plan::tracing::{PlanTracePolicy, TracePolicy};
use crate::policy::gc_work::{TraceKind, TRACE_KIND_TRANSITIVE_PIN};
use crate::policy::immix::TRACE_KIND_FAST;
use crate::vm::VMBinding;

pub(super) struct ConcurrentImmixSTWGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ConcurrentImmixSTWGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = ConcurrentImmix<VM>;
    type DefaultTracePolicy = PlanTracePolicy<ConcurrentImmix<VM>, KIND>;
    type PinningTracePolicy = PlanTracePolicy<ConcurrentImmix<VM>, TRACE_KIND_TRANSITIVE_PIN>;
}
pub(super) struct ConcurrentImmixGCWorkContext<T: TracePolicy>(std::marker::PhantomData<T>);

impl<T: TracePolicy> crate::scheduler::GCWorkContext for ConcurrentImmixGCWorkContext<T> {
    type VM = T::VM;
    type PlanType = ConcurrentImmix<T::VM>;
    type DefaultTracePolicy = T;
    type PinningTracePolicy = T;

    fn make_roots_work_factory(
        mmtk: &'static crate::MMTK<Self::VM>,
    ) -> impl crate::vm::RootsWorkFactory<<Self::VM as VMBinding>::VMSlot> {
        ConcurrentMarkingRootsWorkFactory::<Self::VM, Self::PlanType, TRACE_KIND_FAST>::new(mmtk)
    }
}
