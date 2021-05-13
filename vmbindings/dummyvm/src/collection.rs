use mmtk::vm::Collection;
use mmtk::MutatorContext;
use mmtk::util::OpaquePointer;
use mmtk::scheduler::*;
use DummyVM;

pub struct VMCollection {}

impl Collection<DummyVM> for VMCollection {
    fn stop_all_mutators<E: ProcessEdgesWork<VM=DummyVM>>(_tls: OpaquePointer) {
        unimplemented!()
    }

    fn resume_mutators(_tls: OpaquePointer) {
        unimplemented!()
    }

    fn block_for_gc(_tls: OpaquePointer) {
        unimplemented!();
    }

    fn spawn_worker_thread(_tls: OpaquePointer, _ctx: Option<&GCWorker<DummyVM>>) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext<DummyVM>>(_tls_w: OpaquePointer, _tls_m: OpaquePointer, _mutator: &T) {
        unimplemented!()
    }
}