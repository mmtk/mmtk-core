use ::plan::{Plan, SelectedPlan};
use super::super::ActivePlan;
use ::util::OpaquePointer;
use libc::c_void;
use vm::dummyvm::DummyVM;

pub struct VMActivePlan<> {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static SelectedPlan<DummyVM> {
        &::mmtk::SINGLETON.plan
    }

    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(tls: OpaquePointer) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::MutatorT> {
        unimplemented!()
    }
}