use mmtk::{Plan, SelectedPlan};
use mmtk::vm::ActivePlan;
use mmtk::util::OpaquePointer;
use mmtk::scheduler::*;
use DummyVM;
use SINGLETON;

pub struct VMActivePlan<> {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static SelectedPlan<DummyVM> {
        &SINGLETON.plan
    }

    unsafe fn worker(_tls: OpaquePointer) -> &'static mut GCWorker<DummyVM> {
        unimplemented!()
    }

    fn number_of_mutators() -> usize {
        unimplemented!()
    }

    unsafe fn is_mutator(_tls: OpaquePointer) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(_tls: OpaquePointer) -> &'static mut <SelectedPlan<DummyVM> as Plan>::Mutator {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<DummyVM> as Plan>::Mutator> {
        unimplemented!()
    }
}