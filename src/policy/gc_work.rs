/// Used to identify the trace if a policy has different kinds of traces. For example, defrag vs fast trace for Immix,
/// mark vs forward trace for mark compact.
pub(crate) type TraceKind = u8;

pub const DEFAULT_TRACE: u8 = u8::MAX;

use crate::plan::TransitiveClosure;
use crate::scheduler::GCWork;
use crate::util::copy::CopySemantics;
use crate::vm::VMBinding;
use crate::util::ObjectReference;
use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::GCWorker;

pub trait PolicyTraceObject<VM: VMBinding> {
    fn trace_object<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        copy: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;
    #[inline(always)]
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>> {
        Box::new(crate::scheduler::gc_work::ScanObjects::<E>::new(
            nodes, false,
        ))
    }
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}
