use ::plan::plan::Plan;

pub trait ActivePlan<T: Plan> {
    fn global() -> T;
    fn collector() -> T::CollectorT;
    fn is_mutator() -> bool;
    fn mutator() -> T::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator();
}