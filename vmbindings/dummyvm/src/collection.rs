use mmtk::vm::Collection;
use mmtk::{MutatorContext, ParallelCollector};
use mmtk::util::OpaquePointer;
use DummyVM;

pub struct VMCollection {}

impl Collection<DummyVM> for VMCollection {
    fn stop_all_mutators(_tls: OpaquePointer) {
        unimplemented!()
    }

    fn resume_mutators(_tls: OpaquePointer) {
        unimplemented!()
    }

    fn block_for_gc(_tls: OpaquePointer) {
        unimplemented!();
    }

    fn spawn_worker_thread<T: ParallelCollector<DummyVM>>(_tls: OpaquePointer, _ctx: Option<&mut T>) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext<DummyVM>>(_tls: OpaquePointer, _mutator: &T) {
        unimplemented!()
    }
}