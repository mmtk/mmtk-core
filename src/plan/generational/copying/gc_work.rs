use super::global::GenCopy;
use crate::plan::generational::gc_work::GenNurseryTracePolicy;
use crate::plan::tracing::{PlanTracePolicy, UnsupportedTracePolicy};
use crate::vm::*;

use crate::policy::gc_work::DEFAULT_TRACE;

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultTracePolicy = GenNurseryTracePolicy<Self::VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningTracePolicy = UnsupportedTracePolicy<VM>;
}

pub struct GenCopyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultTracePolicy = PlanTracePolicy<GenCopy<VM>, DEFAULT_TRACE>;
    type PinningTracePolicy = UnsupportedTracePolicy<VM>;
}
