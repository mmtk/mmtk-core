use super::global::SemiSpace;
use crate::plan::tracing::{PlanTracePolicy, UnsupportedTracePolicy};
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::vm::VMBinding;

pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type DefaultTracePolicy = PlanTracePolicy<SemiSpace<VM>, DEFAULT_TRACE>;
    type PinningTracePolicy = UnsupportedTracePolicy<VM>;
}
