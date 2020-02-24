use super::super::Collection;
use ::plan::{MutatorContext, ParallelCollector};
use ::util::OpaquePointer;
use libc::c_void;
use vm::dummyvm::DummyVM;

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

    unsafe fn spawn_worker_thread<T: ParallelCollector<DummyVM>>(tls: OpaquePointer, ctx: *mut T) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T) {
        unimplemented!()
    }
}