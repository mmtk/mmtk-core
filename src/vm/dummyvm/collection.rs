use super::DummyVM;
use crate::util::OpaquePointer;
use crate::vm::Collection;
use crate::{MutatorContext, ParallelCollector};

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

    fn spawn_worker_thread<T: ParallelCollector<DummyVM>>(
        _tls: OpaquePointer,
        _ctx: Option<&mut T>,
    ) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(_tls: OpaquePointer, _m: &T) {
        unimplemented!()
    }
}
