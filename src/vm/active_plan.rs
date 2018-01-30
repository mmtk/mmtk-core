use ::plan::{Plan, SelectedPlan};

pub trait ActivePlan<'a> {
    fn global() -> &'static SelectedPlan<'static>;
    unsafe fn collector(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::CollectorT;
    unsafe fn is_mutator(thread_id: usize) -> bool;
    unsafe fn mutator(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'a mut <SelectedPlan<'a> as Plan>::MutatorT>;
}