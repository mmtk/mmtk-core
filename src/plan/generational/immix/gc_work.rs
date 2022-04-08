use super::global::GenImmix;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::TransitiveClosure;
use crate::policy::gc_work::{PolicyProcessEdges, TraceKind};
use crate::policy::immix::ImmixSpace;
use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST};
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::ProcessEdgesWork;
use crate::util::copy::CopySemantics;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

impl<VM: VMBinding> crate::policy::gc_work::UsePolicyProcessEdges<VM> for GenImmix<VM> {
    type DefaultSpaceType = ImmixSpace<VM>;

    #[inline(always)]
    fn get_target_space(&self) -> &Self::DefaultSpaceType {
        &self.immix
    }

    #[inline(always)]
    fn target_trace<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if KIND == TRACE_KIND_FAST {
            self.immix.fast_trace_object(trace, object)
        } else {
            self.immix
                .trace_object(trace, object, CopySemantics::Mature, worker)
        }
    }

    #[inline(always)]
    fn may_move_objects<const KIND: TraceKind>() -> bool {
        KIND == TRACE_KIND_DEFRAG
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

    #[inline(always)]
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>> {
        Box::new(crate::policy::immix::ScanObjectsAndMarkLines::<E>::new(
            nodes,
            false,
            &self.immix,
        ))
    }
}

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
    type ProcessEdgesWorkType = PolicyProcessEdges<VM, GenImmix<VM>, KIND>;
}
