use crate::plan::concurrent::Pause;
use crate::plan::Plan;

pub trait ConcurrentPlan: Plan {
    fn concurrent_work_in_progress(&self) -> bool;
    fn current_pause(&self) -> Option<Pause>;
}
