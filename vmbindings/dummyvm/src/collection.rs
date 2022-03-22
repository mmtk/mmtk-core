use mmtk::vm::Collection;
use mmtk::vm::GCThreadContext;
use mmtk::MutatorContext;
use mmtk::util::opaque_pointer::*;
use mmtk::scheduler::*;
use crate::DummyVM;

pub struct VMCollection {}

impl Collection<DummyVM> for VMCollection {
    fn stop_all_mutators<E: ProcessEdgesWork<VM=DummyVM>>(_tls: VMWorkerThread) {
        unimplemented!()
    }

    fn resume_mutators(_tls: VMWorkerThread) {
        unimplemented!()
    }

    fn block_for_gc(_tls: VMMutatorThread) {
        panic!("block_for_gc is not implemented")
    }

    fn spawn_gc_thread(_tls: VMThread, _ctx: GCThreadContext<DummyVM>) {

    }

    fn prepare_mutator<T: MutatorContext<DummyVM>>(_tls_w: VMWorkerThread, _tls_m: VMMutatorThread, _mutator: &T) {
        unimplemented!()
    }
}
