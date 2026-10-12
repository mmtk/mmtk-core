use crate::plan::concurrent::Pause;
use crate::plan::Plan;

/// Trait for a concurrent plan.
pub trait ConcurrentPlan: Plan {
    /// Return `true`` if concurrent work (such as concurrent marking) is in progress.
    fn concurrent_work_in_progress(&self) -> bool;
    /// Return the current pause kind.  `None` if not in a pause.
    fn current_pause(&self) -> Option<Pause>;
    /// Called when concurrent work is interrupted.
    fn on_concurrent_work_interrupted(&self);
    /// Called when the concurrent phase's work packets have drained: every GC worker has parked,
    /// so nothing is in progress, and no pause has been requested.
    ///
    /// Return `true` if the plan added more concurrent work (with `add_no_notify`, because the
    /// caller holds the lock that the usual notification needs) or changed its state so that a
    /// pause is now required.  The last parked worker then looks for work again, instead of
    /// parking.
    fn on_concurrent_work_drained(&self) -> bool;
}
