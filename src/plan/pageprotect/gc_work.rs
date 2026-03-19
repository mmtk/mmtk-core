use super::global::PageProtect;
use crate::plan::tracing::PlanTracePolicy;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type DefaultTracePolicy = PlanTracePolicy<PageProtect<VM>, DEFAULT_TRACE>;
    type PinningTracePolicy = PlanTracePolicy<PageProtect<VM>, DEFAULT_TRACE>;
}
