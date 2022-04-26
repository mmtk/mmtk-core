use super::global::SemiSpace;
use crate::scheduler::gc_work::SFTProcessEdges;
use crate::vm::VMBinding;
use crate::policy::gc_work::PlanProcessEdges;
use crate::policy::gc_work::DEFAULT_TRACE;

pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<Self::VM, SemiSpace<VM>, DEFAULT_TRACE>;
}
