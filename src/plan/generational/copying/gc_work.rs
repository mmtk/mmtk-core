use super::global::GenCopy;
use crate::vm::*;
use crate::scheduler::gc_work::SFTProcessEdges;

pub struct GenCopyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type ProcessEdgesWorkType = SFTProcessEdges<Self::VM>;
}
