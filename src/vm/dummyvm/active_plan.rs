use super::DummyVM;
use super::SINGLETON;
use crate::util::OpaquePointer;
use crate::vm::ActivePlan;
use crate::{Plan, SelectedPlan};
use libc::c_void;

pub struct VMActivePlan {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static SelectedPlan<DummyVM> {
        &SINGLETON.plan
    }

    unsafe fn collector(
        tls: OpaquePointer,
    ) -> &'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(tls: OpaquePointer) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(
        tls: OpaquePointer,
    ) -> &'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<DummyVM> as Plan<DummyVM>>::MutatorT>
    {
        unimplemented!()
    }
}
