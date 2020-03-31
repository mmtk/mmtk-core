use crate::plan::{TraceLocal, TransitiveClosure};
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub trait Scanning<VM: VMBinding> {
    fn scan_object<T: TransitiveClosure>(
        trace: &mut T,
        object: ObjectReference,
        tls: OpaquePointer,
    );
    fn reset_thread_counter();
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer);
    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn supports_return_barrier() -> bool;
}
