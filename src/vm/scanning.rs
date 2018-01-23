use ::plan::{TransitiveClosure, TraceLocal};
use ::util::ObjectReference;

pub trait Scanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, thread_id: usize);
    fn reset_thread_counter();
    fn notify_initial_thread_scan_complete(partial_scan: bool, thread_id: usize);
    fn compute_static_roots<T: TraceLocal>(trace: &mut T);
    fn compute_global_roots<T: TraceLocal>(trace: &mut T);
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize);
    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize);
    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T);
    fn supports_return_barrier() -> bool;
}