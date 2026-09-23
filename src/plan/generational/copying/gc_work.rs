use super::global::GenCopy;
use crate::plan::generational::gc_work::GenNurseryTrace;
use crate::plan::tracing::{PlanTrace, UnsupportedTrace};
use crate::vm::*;

use crate::policy::gc_work::DEFAULT_TRACE;

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultTrace = GenNurseryTrace<Self::VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningTrace = UnsupportedTrace<VM>;
}

pub struct GenCopyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultTrace = PlanTrace<GenCopy<VM>, DEFAULT_TRACE>;
    type PinningTrace = UnsupportedTrace<VM>;
}
