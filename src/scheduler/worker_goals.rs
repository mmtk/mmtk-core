//! This module contain "goals" which are larger than work packets, and describes what workers are
//! working towards on a high level.
//!
//! A "goal" is represented by a `WorkerGoal`.  All workers work towards a single goal at a time.
//! The current goal influences the behavior of GC workers, especially the last parked worker.
//! For example,
//!
//! -   When in the progress of GC, the last parker will try to open buckets or announce the GC
//!     has finished.
//! -   When stopping for fork, every waken worker should save its thread state (giving in the
//!     `GCWorker` struct) and exit.
//!
//! The struct `WorkerGoals` keeps the set of goals requested by mutators, but GC workers will only
//! respond to one request at a time, and will favor higher-priority goals.

use enum_map::{Enum, EnumMap};

/// This current and reqeusted goals.
#[derive(Default, Debug)]
pub(crate) struct WorkerGoals {
    /// The current goal.
    current: Option<WorkerGoal>,
    /// Requests received from mutators.  `requests[goal]` is true if the `goal` is requested.
    requests: EnumMap<WorkerGoal, bool>,
}

/// A goal, i.e. something that workers should work together to achieve.
///
/// Members of this `enum` should be listed from the highest priority to the lowest priority.
#[derive(Debug, Enum, Clone, Copy)]
pub(crate) enum WorkerGoal {
    /// Do a garbage collection.
    Gc,
    /// Stop all GC threads so that the VM can call `fork()`.
    StopForFork,
}

impl WorkerGoals {
    /// Set the `goal` as requested.  Return `true` if the requested state of the `goal` changed
    /// from `false` to `true`.
    pub fn set_request(&mut self, goal: WorkerGoal) -> bool {
        if !self.requests[goal] {
            self.requests[goal] = true;
            true
        } else {
            false
        }
    }

    /// Move the highest priority goal from the pending requests to the current request.  Return
    /// that goal, or `None` if no goal has been requested.
    pub fn poll_next_goal(&mut self) -> Option<WorkerGoal> {
        for (goal, requested) in self.requests.iter_mut() {
            if *requested {
                *requested = false;
                self.current = Some(goal);
                probe!(mmtk, goal_set, goal);
                return Some(goal);
            }
        }
        None
    }

    /// Get the current goal if exists.
    pub fn current(&self) -> Option<WorkerGoal> {
        self.current
    }

    /// Called when the current goal is completed.  This will clear the current goal.
    pub fn on_current_goal_completed(&mut self) {
        probe!(mmtk, goal_complete);
        self.current = None
    }

    /// Test if the given `goal` is requested.  Used for debug purpose, only.  The workers always
    /// respond to the request of the highest priority first.
    pub fn debug_is_requested(&self, goal: WorkerGoal) -> bool {
        self.requests[goal]
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerGoal, WorkerGoals};

    #[test]
    fn test_poll_none() {
        let mut goals = WorkerGoals::default();
        let next_goal = goals.poll_next_goal();

        assert!(next_goal.is_none());
        assert!(goals.current().is_none());
    }

    #[test]
    fn test_poll_one() {
        let mut goals = WorkerGoals::default();
        goals.set_request(WorkerGoal::StopForFork);
        let next_goal = goals.poll_next_goal();

        assert!(matches!(next_goal, Some(WorkerGoal::StopForFork)));
        assert!(matches!(goals.current(), Some(WorkerGoal::StopForFork)));
    }

    #[test]
    fn test_goals_priority() {
        let mut goals = WorkerGoals::default();
        goals.set_request(WorkerGoal::StopForFork);
        goals.set_request(WorkerGoal::Gc);

        let next_goal = goals.poll_next_goal();

        assert!(matches!(next_goal, Some(WorkerGoal::Gc)));
        assert!(matches!(goals.current(), Some(WorkerGoal::Gc)));
    }
}
