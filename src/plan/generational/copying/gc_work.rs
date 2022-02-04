use super::global::GenCopy;
use crate::vm::*;

use crate::scheduler::gc_work::MMTkProcessEdges;

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type ProcessEdgesWorkType = MMTkProcessEdges<VM>;
}

pub(super) struct GenCopyMatureGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyMatureGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type ProcessEdgesWorkType = MMTkProcessEdges<VM>;
}
