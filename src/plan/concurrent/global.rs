use crate::plan::concurrent::Pause;
use crate::plan::Plan;

/// Trait for a concurrent plan.
pub trait ConcurrentPlan: Plan {
    /// Return `true`` if concurrent work (such as concurrent marking) is in progress.
    fn concurrent_work_in_progress(&self) -> bool;
    /// Return the current pause kind.  `None` if not in a pause.
    fn current_pause(&self) -> Option<Pause>;
}
