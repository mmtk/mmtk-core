use mmtk::vm::Collection;
use mmtk::MutatorContext;
use mmtk::util::OpaquePointer;
use mmtk::MMTK;
use mmtk::scheduler::*;
use mmtk::scheduler::gc_work::*;
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

    fn spawn_worker_thread(_tls: OpaquePointer, _ctx: Option<&Worker<MMTK<DummyVM>>>) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext<DummyVM>>(_tls_w: OpaquePointer, _tls_m: OpaquePointer, _mutator: &T) {
        unimplemented!()
    }
}