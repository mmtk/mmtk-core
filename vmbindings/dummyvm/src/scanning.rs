use mmtk::vm::Scanning;
use mmtk::{TransitiveClosure, TraceLocal};
use mmtk::util::{ObjectReference, SynchronizedCounter};
use mmtk::util::OpaquePointer;
use crate::DummyVM;

static COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMScanning {}

impl Scanning<DummyVM> for VMScanning {
    fn scan_object<T: TransitiveClosure>(_trace: &mut T, _object: ObjectReference, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn reset_thread_counter() {
        COUNTER.reset();
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_static_roots<T: TraceLocal>(_trace: &mut T, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_global_roots<T: TraceLocal>(_trace: &mut T, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_thread_roots<T: TraceLocal>(_trace: &mut T, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_new_thread_roots<T: TraceLocal>(_trace: &mut T, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn compute_bootimage_roots<T: TraceLocal>(_trace: &mut T, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}