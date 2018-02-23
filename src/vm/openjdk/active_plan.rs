use ::plan::{Plan, SelectedPlan};
use super::super::ActivePlan;

pub struct VMActivePlan<> {}

impl ActivePlan for VMActivePlan {
    fn global() -> &'static SelectedPlan {
        unimplemented!()
    }

    unsafe fn collector(thread_id: usize) -> &'static mut <SelectedPlan as Plan>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(thread_id: usize) -> bool {
        unimplemented!()
    }

    unsafe fn mutator(thread_id: usize) -> &'static mut <SelectedPlan as Plan>::MutatorT {
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