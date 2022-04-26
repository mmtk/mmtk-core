use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::TransitiveClosure;
// use crate::policy::gc_work::{PolicyProcessEdges, TraceKind};
use crate::policy::gc_work::TraceKind;
use crate::policy::immix::ImmixSpace;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

impl<VM: VMBinding> crate::policy::gc_work::UsePolicyProcessEdges<VM> for GenImmix<VM> {
    type TargetPolicy = ImmixSpace<VM>;
    const COPY: CopySemantics = CopySemantics::Mature;

    #[inline(always)]
    fn get_target_space(&self) -> &Self::TargetPolicy {
        &self.immix
    }

    #[inline(always)]
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.gen.trace_object_full_heap::<T>(trace, object, worker)
    }
}

use crate::policy::gc_work::PlanProcessEdges;
use crate::policy::gc_work::DEFAULT_TRACE;

pub struct GenImmixNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenImmixNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM>;
}

pub(super) struct GenImmixMatureGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for GenImmixMatureGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = GenImmix<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<VM, GenImmix<VM>, KIND>;
}
