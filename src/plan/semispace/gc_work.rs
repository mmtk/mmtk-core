use super::global::SemiSpace;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::gc_work::{PlanProcessEdges, UnsupportedProcessEdges};
use crate::vm::VMBinding;

pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type DefaultProcessEdges = PlanProcessEdges<Self::VM, SemiSpace<VM>, DEFAULT_TRACE>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}
