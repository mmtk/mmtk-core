use ::plan::{Plan, SelectedPlan};

pub trait ActivePlan {
    fn global() -> &'static SelectedPlan { &::plan::selected_plan::PLAN }
    unsafe fn collector(thread_id: usize) -> &'static mut <SelectedPlan as Plan>::CollectorT;
    unsafe fn is_mutator(thread_id: usize) -> bool;
    unsafe fn mutator(thread_id: usize) -> &'static mut <SelectedPlan as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan as Plan>::MutatorT>;
}