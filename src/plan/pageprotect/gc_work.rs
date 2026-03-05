use super::global::PageProtect;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::gc_work::PlanProcessSlots;
use crate::vm::VMBinding;

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type DefaultProcessEdges = PlanProcessSlots<Self::VM, PageProtect<VM>, DEFAULT_TRACE>;
    type PinningProcessEdges = PlanProcessSlots<Self::VM, PageProtect<VM>, DEFAULT_TRACE>;
}
