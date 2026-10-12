use crate::plan::concurrent::span::global::ConcurrentSpan;
use crate::plan::tracing::PlanTrace;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

/// The `GCWorkContext` for stop-the-world `Pause::Full` collections in `ConcurrentSpan`.
pub(super) struct ConcurrentSpanSTWGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);

impl<VM: VMBinding> crate::scheduler::GCWorkContext for ConcurrentSpanSTWGCWorkContext<VM> {
    type VM = VM;
    type PlanType = ConcurrentSpan<VM>;
    type DefaultTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
    type PinningTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
}
