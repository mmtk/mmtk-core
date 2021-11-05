use crate::DummyVM;
use mmtk::scheduler::*;
use mmtk::util::opaque_pointer::*;
use mmtk::util::ObjectReference;
use mmtk::vm::Scanning;
use mmtk::{Mutator, TransitiveClosure};

pub struct VMScanning {}

impl Scanning<DummyVM> for VMScanning {
    fn scan_objects<W: ProcessEdgesWork<VM = DummyVM>>(
        _objects: &[ObjectReference],
        _worker: &mut GCWorker<DummyVM>,
    ) {
        unimplemented!()
    }
    fn scan_thread_roots<W: ProcessEdgesWork<VM = DummyVM>>() {
        unimplemented!()
    }
    fn scan_thread_root<W: ProcessEdgesWork<VM = DummyVM>>(
        _mutator: &'static mut Mutator<DummyVM>,
        _tls: VMWorkerThread,
    ) {
        unimplemented!()
    }
    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM = DummyVM>>() {
        unimplemented!()
    }
    fn scan_object<T: TransitiveClosure>(
        _trace: &mut T,
        _object: ObjectReference,
        _tls: VMWorkerThread,
    ) {
        unimplemented!()
    }
    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: VMWorkerThread) {
        unimplemented!()
    }
    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
    fn prepare_for_roots_re_scanning() {
        unimplemented!()
    }
}
