use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal};
use ::util::ObjectReference;

pub struct JikesRVMScanning {}

impl Scanning for JikesRVMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference) {
        unimplemented!()
    }

    fn reset_thread_counter() {
        unimplemented!()
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool) {
        unimplemented!()
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T) {
        Self::compute_thread_roots(trace, false)
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T) {
        Self::compute_thread_roots(trace, true)
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}

impl JikesRVMScanning {
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, new_roots_sufficient: bool) {
        unimplemented!()
    }
}