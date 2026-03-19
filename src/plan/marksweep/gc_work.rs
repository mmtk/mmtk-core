use super::MarkSweep;
use crate::plan::tracing::PlanTracePolicy;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct MSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkSweep<VM>;
    type DefaultTracePolicy = PlanTracePolicy<MarkSweep<VM>, DEFAULT_TRACE>;
    type PinningTracePolicy = PlanTracePolicy<MarkSweep<VM>, DEFAULT_TRACE>;
}
