use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal};
use ::util::ObjectReference;

pub struct OpenJDKScanning {}

impl Scanning for OpenJDKScanning {
    fn scan_object<T: TransitiveClosure>(trace: T, object: ObjectReference) {
        unimplemented!()
    }

    fn reset_thread_counter() {
        unimplemented!()
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool) {
        unimplemented!()
    }

    fn compute_static_roots<T: TraceLocal>(trace: T) {
        unimplemented!()
    }

    fn compute_global_roots<T: TraceLocal>(trace: T) {
        unimplemented!()
    }

    fn compute_thread_roots<T: TraceLocal>(trace: T) {
        unimplemented!()
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: T) {
        unimplemented!()
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: T) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}