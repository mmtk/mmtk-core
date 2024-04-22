use super::MarkSweep;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::gc_work::*;
use crate::vm::VMBinding;

pub struct MSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkSweep<VM>;
    type DefaultProcessEdges = PlanProcessEdges<Self::VM, MarkSweep<VM>, DEFAULT_TRACE>;
    type PinningProcessEdges = PlanProcessEdges<Self::VM, MarkSweep<VM>, DEFAULT_TRACE>;
}
