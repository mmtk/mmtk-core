/// Used to identify the trace if a policy has different kinds of traces. For example, defrag vs fast trace for Immix,
/// mark vs forward trace for mark compact.
pub(crate) type TraceKind = u8;

pub const DEFAULT_TRACE: u8 = u8::MAX;

use crate::plan::TransitiveClosure;
use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

use crate::util::ObjectReference;

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

    /// Policy-specific post-scan-object hook.  It is called after scanning
    /// each object in this space.
    #[inline(always)]
    fn post_scan_object(&self, _object: ObjectReference) {
        // Do nothing.
    }

    /// Return whether the policy moves objects.
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}
