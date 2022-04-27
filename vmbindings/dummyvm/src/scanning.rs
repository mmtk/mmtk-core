use crate::DummyVM;
use mmtk::scheduler::*;
use mmtk::util::opaque_pointer::*;
use mmtk::util::ObjectReference;
use mmtk::vm::EdgeVisitor;
use mmtk::vm::Scanning;
use mmtk::Mutator;

pub struct VMScanning {}

impl Scanning<DummyVM> for VMScanning {
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
    fn scan_object<EV: EdgeVisitor>(
        _tls: VMWorkerThread,
        _object: ObjectReference,
        _edge_visitor: &mut EV,
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
