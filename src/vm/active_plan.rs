use ::plan::plan::Plan;

pub trait ActivePlan<T: Plan> {
    fn global(thread_id: usize) -> T;
    fn collector(thread_id: usize) -> T::CollectorT;
    fn is_mutator(thread_id: usize) -> bool;
    fn mutator(thread_id: usize) -> T::MutatorT;
    fn collector_count(thread_id: usize) -> usize;
    fn reset_mutator_iterator(thread_id: usize);
    fn get_next_mutator(thread_id: usize);
}