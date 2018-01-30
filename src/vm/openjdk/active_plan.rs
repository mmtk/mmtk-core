use ::plan::{Plan, SelectedPlan};
use super::super::ActivePlan;

pub struct VMActivePlan<> {}

impl<'a> ActivePlan<'a> for VMActivePlan {
    fn global() -> &'static SelectedPlan<'static> {
        unimplemented!()
    }

    fn collector(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::CollectorT {
        unimplemented!()
    }

    fn is_mutator(thread_id: usize) -> bool {
        unimplemented!()
    }

    fn mutator(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::MutatorT {
        unimplemented!()
    }

    fn collector_count(thread_id: usize) -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator(thread_id: usize) {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'a mut <SelectedPlan<'a> as Plan>::MutatorT> {
        unimplemented!()
    }
}