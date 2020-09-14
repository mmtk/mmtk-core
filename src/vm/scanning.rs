use crate::plan::{TraceLocal, TransitiveClosure};
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::work::*;

pub trait Scanning<VM: VMBinding> {
    fn scan_object<T: TransitiveClosure>(
        trace: &mut T,
        object: ObjectReference,
        tls: OpaquePointer,
    );
    fn reset_thread_counter();
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer);
    /// Scan all thread roots and create `RootsEdge` work packets
    ///
    /// TODO: Smaller work granularity
    fn scan_thread_roots<W: ProcessEdgesWork<VM=VM>>(tls: OpaquePointer);
    fn scan_objects<W: ProcessEdgesWork<VM=VM>>(objects: &[ObjectReference]);
    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn supports_return_barrier() -> bool;
}
