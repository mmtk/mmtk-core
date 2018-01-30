use ::plan::{Plan, SelectedPlan};
use super::super::ActivePlan;

pub struct VMActivePlan<> {}

impl<'a> ActivePlan<'a> for VMActivePlan {
    fn global() -> &'static SelectedPlan<'static> {
        unimplemented!()
    }

    unsafe fn collector(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(thread_id: usize) -> bool {
        unimplemented!()
    }

    unsafe fn mutator(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'a mut <SelectedPlan<'a> as Plan>::MutatorT> {
        unimplemented!()
    }
}