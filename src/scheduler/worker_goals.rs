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
#[derive(Default, Debug)] // All fields should be false by default.
pub(crate) struct WorkerRequests {
    /// The VM needs to fork.  Workers should save their contexts and exit.
    pub(crate) stop_for_fork: bool,
    /// GC is requested.  Workers should schedule a GC.
    pub(crate) gc: bool,
}
