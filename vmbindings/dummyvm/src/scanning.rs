use mmtk::vm::Scanning;
use mmtk::{TransitiveClosure, SelectedPlan, Mutator};
use mmtk::util::{ObjectReference, SynchronizedCounter};
use mmtk::util::OpaquePointer;
use mmtk::scheduler::gc_works::*;
use crate::DummyVM;

static COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMScanning {}

impl Scanning<DummyVM> for VMScanning {
    fn scan_objects<W: ProcessEdgesWork<VM=DummyVM>>(_objects: &[ObjectReference]) {
        unimplemented!()
    }
    fn scan_thread_roots<W: ProcessEdgesWork<VM=DummyVM>>() {
        unimplemented!()
    }
    fn scan_thread_root<W: ProcessEdgesWork<VM=DummyVM>>(_mutator: &'static mut Mutator<SelectedPlan<DummyVM>>) {
        unimplemented!()
    }
    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM=DummyVM>>() {
        unimplemented!()
    }
    fn scan_object<T: TransitiveClosure>(_trace: &mut T, _object: ObjectReference, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn reset_thread_counter() {
        COUNTER.reset();
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: OpaquePointer) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}