use libc::c_void;
use mmtk::vm::Scanning;
use mmtk::{TransitiveClosure, TraceLocal};
use mmtk::util::{ObjectReference, SynchronizedCounter};
use mmtk::util::OpaquePointer;
use OpenJDK;
use super::UPCALLS;

static COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMScanning {}

impl Scanning<OpenJDK> for VMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).scan_object)(::std::mem::transmute(trace), object, tls);
        }
    }

    fn reset_thread_counter() {
        COUNTER.reset();
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer) {
        // unimplemented!()
        // TODO
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).compute_static_roots)(::std::mem::transmute(trace), tls);
        }
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).compute_global_roots)(::std::mem::transmute(trace), tls);
        }
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).compute_thread_roots)(::std::mem::transmute(trace), tls);
        }
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        // Do nothing
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}