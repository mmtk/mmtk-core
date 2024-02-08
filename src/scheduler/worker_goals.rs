//! This module contain "goals" which are larger than work packets, and describes what workers are
//! working towards on a high level.
//!
//! A "goal" is represented by a `WorkerGoal`.  All workers work towards a single goal at a time.
//! THe current goal influences the behavior of GC workers, especially the last parked worker.
//! For example,
//!
//! -   When in the progress of GC, the last parker will try to open buckets or announce the GC
//!     has finished.
//! -   When stopping for fork, every worker should exit when waken.
//!
//! The struct `WorkerRequests` keeps a list of requests from mutators, such as requests for GC
//! and requests for forking.  But the GC workers will only respond to one request at a time.

use std::time::Instant;

/// This current and reqeusted goals.
#[derive(Default, Debug)]
pub(crate) struct WorkerGoals {
    /// What are the workers doing now?
    pub(crate) current: Option<WorkerGoal>,
    /// Requests received from mutators.
    pub(crate) requests: WorkerRequests,
}

/// The thing workers are currently doing.  This affects several things, such as what the last
/// parked worker will do, and whether workers will stop themselves.
#[derive(Debug)]
pub(crate) enum WorkerGoal {
    Gc {
        start_time: Instant,
    },
    #[allow(unused)] // TODO: Implement forking support later.
    StopForFork,
}

/// Reqeusts received from mutators.  Workers respond to those requests when they do not have a
/// current goal.  Multiple things can be requested at the same time, and workers respond to the
/// thing with the highest priority.
///
/// The fields of this structs are ordered with decreasing priority.
#[derive(Default, Debug)]
pub(crate) struct WorkerRequests {
    /// The VM needs to fork.  Workers should save their contexts and exit.
    pub(crate) stop_for_fork: WorkerRequest,
    /// GC is requested.  Workers should schedule a GC.
    pub(crate) gc: WorkerRequest,
}

/// To record whether a specific goal has been reqeuested.
/// It is basically a wrapper of `bool`, but forces it to be accessed in a particular way.
#[derive(Default, Debug)] // Default: False by default.
pub(crate) struct WorkerRequest {
    /// True if the goal has been requested.
    requested: bool,
}

impl WorkerRequest {
    /// Set the goal as requested.  Return `true` if its requested state changed from `false` to
    /// `true`.
    pub fn set(&mut self) -> bool {
        if !self.requested {
            self.requested = true;
            true
        } else {
            false
        }
    }

    /// Get the requested state and clear it.  Return `true` if the requested state was `true`.
    pub fn poll(&mut self) -> bool {
        if self.requested {
            self.requested = false;
            true
        } else {
            false
        }
    }

    /// Test if the request is set.  For debug only.  The last parked worker should use `poll` to
    /// get the state and clear it.
    pub fn debug_is_set(&self) -> bool {
        self.requested
    }
}
