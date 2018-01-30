use ::plan::{Plan, SelectedPlan};

pub trait ActivePlan<'a> {
    fn global() -> &'static SelectedPlan<'static>;
    fn collector(thread_id: usize) -> &'a <SelectedPlan<'a> as Plan>::CollectorT;
    fn is_mutator(thread_id: usize) -> bool;
    fn mutator(thread_id: usize) -> &'a <SelectedPlan<'a> as Plan>::MutatorT;
    fn collector_count(thread_id: usize) -> usize;
    fn reset_mutator_iterator(thread_id: usize);
    fn get_next_mutator(thread_id: usize);
}