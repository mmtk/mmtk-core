use super::global::PageProtect;
use crate::plan::tracing::PlanTrace;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type DefaultTrace = PlanTrace<PageProtect<VM>, DEFAULT_TRACE>;
    type PinningTrace = PlanTrace<PageProtect<VM>, DEFAULT_TRACE>;
}
