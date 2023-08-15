use crate::DummyVM;
use mmtk::util::opaque_pointer::*;
use mmtk::vm::Collection;
use mmtk::vm::GCThreadContext;
use mmtk::Mutator;

pub struct VMCollection {}

impl Collection<DummyVM> for VMCollection {
    fn stop_all_mutators<F>(_tls: VMWorkerThread, _mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<DummyVM>),
    {
        unimplemented!()
    }

    fn resume_mutators(_tls: VMWorkerThread) {
        unimplemented!()
    }

    fn block_for_gc(_tls: VMMutatorThread) {
        panic!("block_for_gc is not implemented")
    }

    fn spawn_gc_thread(_tls: VMThread, _ctx: GCThreadContext<DummyVM>) {}
}
