use ::plan::{Plan, SelectedPlan};
use super::super::ActivePlan;

use libc::c_void;

pub struct VMActivePlan<> {}

impl ActivePlan for VMActivePlan {
    unsafe fn collector(tls: *mut c_void) -> &'static mut <SelectedPlan as Plan>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(tls: *mut c_void) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(tls: *mut c_void) -> &'static mut <SelectedPlan as Plan>::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan as Plan>::MutatorT> {
        unimplemented!()
    }
}