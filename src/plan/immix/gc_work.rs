use super::global::Immix;
use crate::policy::gc_work::{PolicyProcessEdges, TraceKind};
use crate::vm::VMBinding;
use crate::scheduler::GCWork;
use crate::policy::immix::ImmixSpace;
use crate::util::copy::CopySemantics;
use crate::plan::TransitiveClosure;
use crate::policy::immix::{TRACE_KIND_FAST, TRACE_KIND_DEFRAG};
use crate::util::ObjectReference;
use crate::scheduler::GCWorker;
use crate::scheduler::ProcessEdgesWork;

impl<VM: VMBinding> crate::policy::gc_work::UsePolicyProcessEdges<VM> for Immix<VM> {
    type SpaceType = ImmixSpace<VM>;

    #[inline(always)]
    fn get_target_space(&'static self) -> &'static Self::SpaceType {
        &self.immix_space
    }
    #[inline(always)]
    fn get_target_copy_semantics<const KIND: TraceKind>() -> CopySemantics {
        CopySemantics::DefaultCopy
    }
    /// How to trace object in target space
    fn target_trace<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if KIND == TRACE_KIND_FAST {
            self.immix_space.fast_trace_object(trace, object)
        } else {
            self.immix_space.trace_object(trace, object, CopySemantics::DefaultCopy, worker)
        }
    }
    fn overwrite_reference<const KIND: TraceKind>() -> bool {
        KIND == TRACE_KIND_DEFRAG
    }
    /// How to trace objects if the object is not in the immix space.
    fn fallback_trace<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.common.trace_object::<T>(trace, object)
    }

    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(&'static self, nodes: Vec<ObjectReference>) -> Box<dyn GCWork<VM>> {
        Box::new(crate::policy::immix::ScanObjectsAndMarkLines::<E>::new(
            nodes,
            false,
            &self.immix_space,
        ))
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
