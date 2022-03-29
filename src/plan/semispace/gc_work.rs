use super::global::SemiSpace;
use crate::scheduler::gc_work::SFTProcessEdges;
use crate::vm::VMBinding;

pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type ProcessEdgesWorkType = SFTProcessEdges<Self::VM>;
}
