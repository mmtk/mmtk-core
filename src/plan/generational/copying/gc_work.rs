use super::global::GenCopy;
use crate::scheduler::gc_work::SFTProcessEdges;
use crate::vm::*;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;

use crate::policy::gc_work::PlanProcessEdges;
use crate::policy::gc_work::DEFAULT_TRACE;

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<Self::VM>;
}

pub struct GenCopyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<Self::VM, GenCopy<VM>, DEFAULT_TRACE>;
}
