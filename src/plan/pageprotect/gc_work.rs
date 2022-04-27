use super::global::PageProtect;
use crate::vm::VMBinding;
use crate::plan::transitive_closure::PlanProcessEdges;
use crate::policy::gc_work::DEFAULT_TRACE;

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<Self::VM, PageProtect<VM>, DEFAULT_TRACE>;
}
