use super::global::GenCopy;
use crate::plan::generational::gc_work::GenNurseryEdgeTracer;
use crate::plan::tracing::{PlanEdgeTracer, UnsupportedEdgeTracer};
use crate::vm::*;

use crate::policy::gc_work::DEFAULT_TRACE;

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultEdgeTracer = GenNurseryEdgeTracer<Self::VM, Self::PlanType, DEFAULT_TRACE>;
    type PinningEdgeTracer = UnsupportedEdgeTracer<VM>;
}

pub struct GenCopyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type DefaultEdgeTracer = PlanEdgeTracer<GenCopy<VM>, DEFAULT_TRACE>;
    type PinningEdgeTracer = UnsupportedEdgeTracer<VM>;
}
