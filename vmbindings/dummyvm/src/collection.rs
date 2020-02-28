use libc::c_void;
use mmtk::vm::Collection;
use mmtk::{MutatorContext, ParallelCollector};
use mmtk::util::OpaquePointer;
use DummyVM;

pub struct VMCollection {}

impl Collection<DummyVM> for VMCollection {
    fn stop_all_mutators(tls: OpaquePointer) {
        unimplemented!()
    }

    fn resume_mutators(tls: OpaquePointer) {
        unimplemented!()
    }

    fn block_for_gc(tls: OpaquePointer) {
        unimplemented!();
    }

    fn spawn_worker_thread<T: ParallelCollector<DummyVM>>(tls: OpaquePointer, ctx: Option<&mut T>) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T) {
        unimplemented!()
    }
}