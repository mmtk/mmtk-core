use ::vm::ActivePlan;
use ::plan::Plan;

pub struct VMActivePlan<> {}

impl<T: Plan> ActivePlan<T> for VMActivePlan {
    fn global(thread_id: usize) -> T {
        unimplemented!()
    }

    fn collector(thread_id: usize) -> T::CollectorT {
        unimplemented!()
    }

    fn is_mutator(thread_id: usize) -> bool {
        unimplemented!()
    }

    fn mutator(thread_id: usize) -> T::MutatorT {
        unimplemented!()
    }

    fn collector_count(thread_id: usize) -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator(thread_id: usize) {
        unimplemented!()
    }

    fn get_next_mutator(thread_id: usize) {
        unimplemented!()
    }
}