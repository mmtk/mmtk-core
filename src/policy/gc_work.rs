/// Used to identify the trace if a policy has different kinds of traces. For example, defrag vs fast trace for Immix,
/// mark vs forward trace for mark compact.
pub(crate) type TraceKind = u8;

pub const DEFAULT_TRACE: u8 = u8::MAX;

use crate::plan::TransitiveClosure;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::util::ObjectReference;
use crate::vm::EdgeVisitor;
use crate::vm::VMBinding;

/// This trait defines policy-specific behavior for tracing objects.
/// The procedural macro #[derive(PlanTraceObject)] will generate code
/// that uses this trait. We expect any policy to implement this trait.
/// For the sake of performance, the implementation
/// of this trait should mark methods as `[inline(always)]`.
pub trait PolicyTraceObject<VM: VMBinding> {
    /// Trace object in the policy. If the policy copies objects, we should
    /// expect `copy` to be a `Some` value.
    fn trace_object<T: TransitiveClosure, const KIND: TraceKind>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        copy: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Policy-specific scan object. The implementation needs to guarantee that
    /// they will call `VM::VMScanning::scan_object()` (or `Self::vm_scan_object()`) besides any space-specific work for the object.
    #[inline(always)]
    fn scan_object<EV: EdgeVisitor>(
        &self,
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut EV,
    ) {
        use crate::vm::Scanning;
        VM::VMScanning::scan_object(tls, object, edge_visitor)
    }

    /// Return whether the policy moves objects.
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}
