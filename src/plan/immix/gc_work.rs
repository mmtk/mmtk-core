use super::global::Immix;
use crate::plan::TransitiveClosure;
use crate::policy::gc_work::{PolicyProcessEdges, TraceKind};
use crate::policy::immix::ImmixSpace;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

impl<VM: VMBinding> crate::policy::gc_work::UsePolicyProcessEdges<VM> for Immix<VM> {
    type TargetPolicy = ImmixSpace<VM>;
    const COPY: CopySemantics = CopySemantics::DefaultCopy;

    #[inline(always)]
    fn get_target_space(&self) -> &Self::TargetPolicy {
        &self.immix_space
    }

    #[inline(always)]
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.common.trace_object::<T>(trace, object)
    }
}

pub(super) struct ImmixGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ImmixGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = Immix<VM>;
    type ProcessEdgesWorkType = PolicyProcessEdges<VM, Immix<VM>, KIND>;
}
