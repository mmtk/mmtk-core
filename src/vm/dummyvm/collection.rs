use super::DummyVM;
use crate::util::OpaquePointer;
use crate::vm::Collection;
use crate::{MutatorContext, ParallelCollector};
use libc::c_void;

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
