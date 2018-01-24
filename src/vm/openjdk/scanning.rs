use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal};
use ::util::ObjectReference;

pub struct VMScanning {}

impl Scanning for VMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, thread_id: usize) {
        unimplemented!()
    }

    fn reset_thread_counter() {
        unimplemented!()
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool, thread_id: usize) {
        unimplemented!()
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}