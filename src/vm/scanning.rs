use ::plan::{TransitiveClosure, TraceLocal};
use ::util::ObjectReference;

use libc::c_void;

pub trait Scanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, tls: *mut c_void);
    fn reset_thread_counter();
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: *mut c_void);
    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: *mut c_void);
    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: *mut c_void);
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: *mut c_void);
    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: *mut c_void);
    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: *mut c_void);
    fn supports_return_barrier() -> bool;
}