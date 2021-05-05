use mmtk::Plan;
use mmtk::vm::ActivePlan;
use mmtk::util::opaque_pointer::*;
use mmtk::scheduler::*;
use mmtk::Mutator;
use DummyVM;
use SINGLETON;

pub struct VMActivePlan<> {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static dyn Plan<VM=DummyVM> {
        &*SINGLETON.plan
    }

    fn worker(_tls: VMWorkerThread) -> &'static mut GCWorker<DummyVM> {
        unimplemented!()
    }

    fn number_of_mutators() -> usize {
        unimplemented!()
    }

    fn is_mutator(_tls: VMThread) -> bool {
        // FIXME
        true
    }

    fn mutator(_tls: VMMutatorThread) -> &'static mut Mutator<DummyVM> {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut Mutator<DummyVM>> {
        unimplemented!()
    }
}