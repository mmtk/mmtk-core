//! This module contains `WorkerMonitor` and related types.  It purposes includes:
//!
//! -   allowing workers to park,
//! -   letting the last parked worker take action, and
//! -   letting workers and mutators to notify workers when workers are given things to do.

use std::sync::{Condvar, Mutex};

use crate::vm::VMBinding;

use super::{
    worker::WorkerShouldExit,
    worker_goals::{WorkerGoal, WorkerGoals},
    GCWorker,
};

/// The result type of the `on_last_parked` call-back in `WorkMonitor::park_and_wait`.
/// It decides how many workers should wake up after `on_last_parked`.
pub(crate) enum LastParkedResult {
    /// The last parked worker should wait, too, until more work packets are added.
    ParkSelf,
    /// The last parked worker should unpark and find work packet to do.
    WakeSelf,
    /// Wake up all parked GC workers.
    WakeAll,
}

/// A data structure for synchronizing workers with each other and with mutators.
///
/// Unlike `GCWorkerShared`, there is only one instance of `WorkerMonitor`.
///
/// -   It allows workers to park and unpark.
/// -   It allows mutators to notify workers to schedule a GC.
pub(crate) struct WorkerMonitor {
    /// The synchronized part.
    sync: Mutex<WorkerMonitorSync>,
    /// Workers wait on this when idle.  Notified if workers have things to do.  That include:
    /// -   any work packets available, and
    /// -   any field in `sync.goals.requests` set to true.
    workers_have_anything_to_do: Condvar,
}

/// The synchronized part of `WorkerMonitor`.
struct WorkerMonitorSync {
    /// Count parked workers.
    parker: WorkerParker,
    /// Current and requested goals.
    goals: WorkerGoals,
}

/// This struct counts the number of workers parked and identifies the last parked worker.
struct WorkerParker {
    /// The total number of workers.
    worker_count: usize,
    /// Number of parked workers.
    parked_workers: usize,
}

impl WorkerParker {
    fn new(worker_count: usize) -> Self {
        Self {
            worker_count,
            parked_workers: 0,
        }
    }

    /// Increase the packed-workers counter.
    /// Called before a worker is parked.
    ///
    /// Return true if all the workers are parked.
    fn inc_parked_workers(&mut self) -> bool {
        let old = self.parked_workers;
        debug_assert!(old < self.worker_count);
        let new = old + 1;
        self.parked_workers = new;
        new == self.worker_count
    }

    /// Decrease the packed-workers counter.
    /// Called after a worker is resumed from the parked state.
    fn dec_parked_workers(&mut self) {
        let old = self.parked_workers;
        debug_assert!(old <= self.worker_count);
        debug_assert!(old > 0);
        let new = old - 1;
        self.parked_workers = new;
    }
}

impl WorkerMonitor {
    pub fn new(worker_count: usize) -> Self {
        Self {
            sync: Mutex::new(WorkerMonitorSync {
                parker: WorkerParker::new(worker_count),
                goals: Default::default(),
            }),
            workers_have_anything_to_do: Default::default(),
        }
    }

    /// Make a request.  Can be called by a mutator to request the workers to work towards the
    /// given `goal`.
    pub fn make_request(&self, goal: WorkerGoal) {
        let mut guard = self.sync.lock().unwrap();
        let newly_requested = guard.goals.set_request(goal);
        if newly_requested {
            self.notify_work_available(false);
        }
    }

    /// Wake up workers when more work packets are made available for workers,
    /// or a mutator has requested the GC workers to schedule a GC.
    pub fn notify_work_available(&self, all: bool) {
        if all {
            self.workers_have_anything_to_do.notify_all();
        } else {
            self.workers_have_anything_to_do.notify_one();
        }
    }

    /// Park a worker and wait on the CondVar `work_available`.
    ///
    /// If it is the last worker parked, `on_last_parked` will be called.
    /// The argument of `on_last_parked` is true if `sync.gc_requested` is `true`.
    /// The return value of `on_last_parked` will determine whether this worker and other workers
    /// will wake up or block waiting.
    ///
    /// This function returns `Ok(())` if the current worker should continue working,
    /// or `Err(WorkerShouldExit)` if the current worker should exit now.
    pub fn park_and_wait<VM, F>(
        &self,
        worker: &GCWorker<VM>,
        on_last_parked: F,
    ) -> Result<(), WorkerShouldExit>
    where
        VM: VMBinding,
        F: FnOnce(&mut WorkerGoals) -> LastParkedResult,
    {
        let mut sync = self.sync.lock().unwrap();

        // Park this worker
        let all_parked = sync.parker.inc_parked_workers();
        trace!(
            "Worker {} parked.  parked/total: {}/{}.  All parked: {}",
            worker.ordinal,
            sync.parker.parked_workers,
            sync.parker.worker_count,
            all_parked
        );

        let mut should_wait = false;

        if all_parked {
            trace!("Worker {} is the last worker parked.", worker.ordinal);
            let result = on_last_parked(&mut sync.goals);
            match result {
                LastParkedResult::ParkSelf => {
                    should_wait = true;
                }
                LastParkedResult::WakeSelf => {
                    // Continue without waiting.
                }
                LastParkedResult::WakeAll => {
                    self.notify_work_available(true);
                }
            }
        } else {
            should_wait = true;
        }

        if should_wait {
            // Notes on CondVar usage:
            //
            // Conditional variables are usually tested in a loop while holding a mutex
            //
            //      lock();
            //      while condition() {
            //          condvar.wait();
            //      }
            //      unlock();
            //
            // The actual condition for this `self.work_available.wait(sync)` is:
            //
            // 1.  any work packet is available, or
            // 2.  a request for scheduling GC is submitted.
            //
            // But it is not used like the typical use pattern shown above, mainly because work
            // packets can be added without holding the mutex `self.sync`.  This means one worker
            // can add a new work packet (no mutex needed) right after another worker finds no work
            // packets are available and then park.  In other words, condition (1) can suddenly
            // become true after a worker sees it is false but before the worker blocks waiting on
            // the CondVar.  If this happens, the last parked worker will block forever and never
            // get notified.  This may happen if mutators or the previously existing "coordinator
            // thread" can add work packets.
            //
            // However, after the "coordinator thread" was removed, only GC worker threads can add
            // work packets during GC.  Parked workers (except the last parked worker) cannot make
            // more work packets availble (by adding new packets or opening buckets).  For this
            // reason, the **last** parked worker can be sure that after it finds no packets
            // available, no other workers can add another work packet (because they all parked).
            // So the **last** parked worker can open more buckets or declare GC finished.
            //
            // Condition (2), i.e. `sync.should_schedule_gc` is guarded by the mutator `sync`.
            // When set (by a mutator via `request_schedule_collection`), it will notify a
            // worker; and the last parked worker always checks it before waiting.  So this
            // condition will not be set without any worker noticing.
            //
            // Note that generational barriers may add `ProcessModBuf` work packets when not in GC.
            // This is benign because those work packets are not executed immediately, and are
            // guaranteed to be executed in the next GC.

            // Notes on spurious wake-up:
            //
            // 1.  The condition variable `work_available` is guarded by `self.sync`.  Because the
            //     last parked worker is holding the mutex `self.sync` when executing
            //     `on_last_parked`, no workers can unpark (even if they spuriously wake up) during
            //     `on_last_parked` because they cannot re-acquire the mutex `self.sync`.
            //
            // 2.  Workers may spuriously wake up and unpark when `on_last_parked` is not being
            //     executed (including the case when the last parked worker is waiting here, too).
            //     If one or more GC workers spuriously wake up, they will check for work packets,
            //     and park again if not available.  The last parked worker will ensure the two
            //     conditions listed above are both false before blocking.  If either condition is
            //     true, the last parked worker will take action.
            sync = self.workers_have_anything_to_do.wait(sync).unwrap();
        }

        // Unpark this worker.
        sync.parker.dec_parked_workers();
        trace!(
            "Worker {} unparked.  parked/total: {}/{}.",
            worker.ordinal,
            sync.parker.parked_workers,
            sync.parker.worker_count,
        );

        // If the current goal is `StopForFork`, the worker thread should exit.
        if matches!(sync.goals.current(), Some(WorkerGoal::StopForFork)) {
            return Err(WorkerShouldExit);
        }

        Ok(())
    }

    /// Called when all workers have exited.
    pub fn on_all_workers_exited(&self) {
        let mut sync = self.sync.try_lock().unwrap();
        sync.goals.on_current_goal_completed();
    }
}
