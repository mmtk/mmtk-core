use ::vm::ActivePlan;
use ::plan::Plan;

pub struct VMActivePlan<> {}

impl<T: Plan> ActivePlan<T> for VMActivePlan {
    fn global() -> T {
        unimplemented!()
    }

    fn collector() -> T::CollectorT {
        unimplemented!()
    }

    fn is_mutator() -> bool {
        unimplemented!()
    }

    fn mutator() -> T::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() {
        unimplemented!()
    }
}