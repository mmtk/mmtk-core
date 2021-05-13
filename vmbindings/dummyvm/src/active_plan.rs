use mmtk::Plan;
use mmtk::vm::ActivePlan;
use mmtk::util::OpaquePointer;
use mmtk::Mutator;
use DummyVM;
use SINGLETON;

pub struct VMActivePlan<> {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static dyn Plan<VM=DummyVM> {
        SINGLETON.get_plan()
    }

    fn number_of_mutators() -> usize {
        unimplemented!()
    }

    unsafe fn is_mutator(_tls: OpaquePointer) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(_tls: OpaquePointer) -> &'static mut Mutator<DummyVM> {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut Mutator<DummyVM>> {
        unimplemented!()
    }
}