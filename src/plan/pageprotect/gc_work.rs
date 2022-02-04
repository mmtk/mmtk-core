use super::global::PageProtect;
use crate::vm::VMBinding;

use crate::scheduler::gc_work::MMTkProcessEdges;

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type ProcessEdgesWorkType = MMTkProcessEdges<VM>;
}
