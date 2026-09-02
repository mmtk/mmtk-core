//! This module contains `WorkerMonitor` and related types.  It purposes includes:
//!
//! -   letting workers become active or inactive,
//! -   letting workers park,
//! -   letting the last parked worker take action, and
//! -   letting workers and mutators notify workers when workers are given things to do.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use super::{
    worker::WorkerShouldExit,
    worker_goals::{WorkerGoal, WorkerGoals},
};

/// The result type of the `on_last_parked` call-back in `WorkMonitor::park_and_wait`.
/// It decides how many workers should wake up after `on_last_parked`.
pub(crate) enum LastParkedResult {
    /// The last parked worker should wait, too, until more work packets are added.
    ParkSelf,
    /// The last parked worker should unpark and find work packet to do.
    WakeSelf,
    /// Unpark the last parked worker and up to `n - 1` others, i.e. just enough workers to cover
    /// `n` available work packets.
    ///
    /// Every work-bucket boundary in a pause runs through here, and several of the buckets an LXR
    /// pause opens hold exactly one packet (`ScheduleCollection`, `StopMutators`,
    /// `ScanMutatorRoots`, `FastRCPrepare`, `Release`). Waking every worker for those meant all
    /// but one immediately found nothing and re-parked, paying for the monitor lock twice more
    /// each. Waking `n` costs one `notify_one` per worker that actually has something to do.
    ///
    /// Under-waking cannot strand work: a worker that adds to an already-open bucket notifies
    /// through [`WorkBucket::add`], so any packet appearing later gets its own wake-up.
    Wake(usize),
    /// Wake up all parked GC workers.  For cases where which worker runs matters (designated
    /// work) or where every worker must observe a new goal (shutdown, fork).
    WakeAll,
}

/// A data structure for synchronizing workers with each other and with mutators.
///
/// Unlike `GCWorkerShared`, there is only one instance of `WorkerMonitor`.
///
/// -   It allows workers to park and unpark.
/// -   It allows mutators to notify workers to schedule a GC.
pub(crate) struct WorkerMonitor {
    /// The total number of workers.
    worker_count: usize,
    /// The synchronized part.
    sync: Mutex<WorkerMonitorSync>,
    /// The number of workers that are allowed to execute work after being notified.
    active_workers: AtomicUsize,
    /// Number of workers currently blocked on either condition variable.
    ///
    /// Maintained under `sync`, but readable without it so that `notify_work_available` can skip
    /// acquiring the lock entirely when nobody is waiting -- the common case while a GC is
    /// running and every worker is busy. See the safety argument on that function.
    sleeping: AtomicUsize,
    /// Active workers wait on this when idle.  A parked *active* worker is notified if workers
    /// have things to do.  That includes:
    /// -   any work packets available, and
    /// -   any field in `sync.goals.requests` set to true.
    workers_have_anything_to_do: Condvar,
    /// Inactive workers wait on this instead.  They are notified whenever the number of active
    /// workers changes, which may turn them into active workers.
    active_worker_number_changed: Condvar,
}

/// The synchronized part of `WorkerMonitor`.
struct WorkerMonitorSync {
    /// Count parked workers.
    parker: WorkerParker,
    /// Current and requested goals.
    goals: WorkerGoals,
    /// Number of workers blocked on `active_worker_number_changed` specifically.
    ///
    /// Only inactive workers wait on that condition variable, which normally means none of them.
    /// Broadcasting to it at every bucket boundary was a wasted syscall per boundary, so the
    /// notify is gated on this being non-zero. It must be a count of actual waiters rather than a
    /// test of `active_workers < worker_count`: raising the active count back up has to wake the
    /// workers waiting to be reactivated, and by then the test would already be false.
    inactive_waiters: usize,
}

/// Whether to keep the old behaviour of waking every worker at every bucket boundary, for
/// measuring what the targeted wake-ups are worth.  Set `MMTK_WAKE_ALL=1`.
///
/// Read once: this sits on the path the targeted wake-ups exist to make cheap, and `var_os`
/// allocates.
fn wake_all_override() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MMTK_WAKE_ALL").is_some())
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
        self.parked_workers = old - 1;
    }
}

impl WorkerMonitor {
    pub fn new(worker_count: usize) -> Self {
        Self {
            worker_count,
            sync: Mutex::new(WorkerMonitorSync {
                parker: WorkerParker::new(worker_count),
                goals: Default::default(),
                inactive_waiters: 0,
            }),
            active_workers: AtomicUsize::new(worker_count),
            sleeping: AtomicUsize::new(0),
            workers_have_anything_to_do: Default::default(),
            active_worker_number_changed: Default::default(),
        }
    }

    pub(crate) fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::SeqCst)
    }

    pub(crate) fn is_worker_active(&self, ordinal: usize) -> bool {
        ordinal < self.active_workers()
    }

    /// Set the number of workers that are allowed to execute work after being notified.
    ///
    /// This only updates the count.  Whenever this changes, the caller must also call
    /// `notify_work_available(true)` (directly, or indirectly via the `on_last_parked`
    /// call-back while still holding the monitor's internal lock) so that both active and
    /// inactive workers wake up and re-evaluate whether they should be active.  We don't do the
    /// notification here because this function may be called while the current thread is the
    /// last parked worker executing `on_last_parked`, in which case the notification has to
    /// happen through the already-held lock instead of acquiring it again.
    pub fn set_active_workers(&self, active_workers: usize) {
        let active_workers = active_workers.clamp(1, self.worker_count);
        self.active_workers.store(active_workers, Ordering::SeqCst);
        debug!(
            "WorkerMonitor active worker count set to {} (worker_count={}).",
            active_workers, self.worker_count
        );
    }

    /// Make a request.  Can be called by a mutator to request the workers to work towards the
    /// given `goal`.
    pub fn make_request(&self, goal: WorkerGoal) {
        // A request always needs all workers, including currently inactive ones, to make
        // progress.
        self.set_active_workers(self.worker_count);
        let mut guard = self.sync.lock().unwrap();
        let newly_requested = guard.goals.set_request(goal);
        if newly_requested {
            self.notify_work_available_while_locked(&guard, true);
        }
    }

    /// Wake up workers when more work packets are made available for workers,
    /// or a mutator has requested the GC workers to schedule a GC.
    pub fn notify_work_available(&self, all: bool) {
        // Fast path: nobody is blocked, so there is nothing to notify and no reason to serialise
        // on the monitor lock.  Every `WorkBucket::add` during a GC comes through here, so this
        // is the difference between one relaxed load and a global lock acquisition per packet.
        //
        // The caller has already made the work visible (pushed it onto the bucket queue) before
        // calling us, and this load is `SeqCst`, so it cannot be reordered before that push.  A
        // worker about to block increments `sleeping` while holding `sync`.  So if we observe
        // zero, any worker that blocks after this point had not yet incremented -- meaning it is
        // still going to acquire `sync`, and it is therefore not yet parked.
        //
        // That worker can only lose this notification if it is the *last* worker to park, and it
        // cannot be: the caller is a running GC worker (it just added a packet), so at least one
        // worker is not parked.  Every non-last parked worker may lose a wake-up harmlessly --
        // the last one to park re-checks for work under the lock and wakes the others.  This is
        // the same invariant the blocking-path comment in `park_and_wait` relies on.
        //
        // Mutators reach this only via generational barriers adding `ProcessModBuf` outside a GC,
        // which that same comment already documents as benign.  Mutators requesting a *goal* go
        // through `make_request`, which still takes the lock.
        if self.sleeping.load(Ordering::SeqCst) == 0 {
            return;
        }
        // We must hold the lock while notifying.  Otherwise a worker that is between checking
        // its wake-up condition and actually blocking on the CondVar (both of which happen while
        // holding this lock) could have the notification delivered too early and lost, causing
        // it to block forever despite the condition it was waiting for having become true.
        let guard = self.sync.lock().unwrap();
        self.notify_work_available_while_locked(&guard, all);
    }

    /// Like `notify_work_available`, but for use by callers that already hold the monitor's
    /// internal lock (such as `park_and_wait` below), to avoid trying to lock it again.
    fn notify_work_available_while_locked(&self, sync: &WorkerMonitorSync, all: bool) {
        if all {
            self.workers_have_anything_to_do.notify_all();
            self.notify_inactive_while_locked(sync);
        } else {
            self.workers_have_anything_to_do.notify_one();
        }
    }

    /// Wake workers waiting to be reactivated, but only if any exist.  See
    /// `WorkerMonitorSync::inactive_waiters`.
    fn notify_inactive_while_locked(&self, sync: &WorkerMonitorSync) {
        if sync.inactive_waiters != 0 {
            self.active_worker_number_changed.notify_all();
        }
    }

    /// Wake `n` workers in total, counting the caller.  See [`LastParkedResult::Wake`].
    ///
    /// Called by the last parked worker while holding `sync`, at which point every other worker is
    /// provably blocked on `workers_have_anything_to_do`: parking requires `sync`, so each of them
    /// has already released it inside `Condvar::wait`.  `notify_one` therefore reliably wakes one
    /// real waiter per call rather than being dropped.
    fn wake_n_while_locked(&self, sync: &WorkerMonitorSync, n: usize) {
        // The caller does not wait, so it covers one of the `n` packets itself.
        let others = n.saturating_sub(1).min(self.worker_count.saturating_sub(1));
        if others >= self.worker_count.saturating_sub(1) {
            // Waking everyone anyway; one broadcast beats N wake-ups.
            self.workers_have_anything_to_do.notify_all();
        } else {
            for _ in 0..others {
                self.workers_have_anything_to_do.notify_one();
            }
        }
        self.notify_inactive_while_locked(sync);
    }

    /// Park a worker and wait on the CondVar `workers_have_anything_to_do`.
    ///
    /// If it is the last worker parked, `on_last_parked` will be called.
    /// The argument of `on_last_parked` is true if `sync.gc_requested` is `true`.
    /// The return value of `on_last_parked` will determine whether this worker and other workers
    /// will wake up or block waiting.
    ///
    /// This function returns `Ok(())` if the current worker should continue working,
    /// or `Err(WorkerShouldExit)` if the current worker should exit now.
    pub fn park_and_wait<F>(
        &self,
        ordinal: usize,
        on_last_parked: F,
    ) -> Result<(), WorkerShouldExit>
    where
        F: FnOnce(&mut WorkerGoals) -> LastParkedResult,
    {
        let mut sync = self.sync.lock().unwrap();

        // Park this worker
        let all_parked = sync.parker.inc_parked_workers();
        trace!(
            "Worker {} parked.  parked/total: {}/{}.  All parked: {}",
            ordinal,
            sync.parker.parked_workers,
            sync.parker.worker_count,
            all_parked
        );

        let mut should_wait = false;

        if all_parked {
            trace!("Worker {} is the last worker parked.", ordinal);
            let result = on_last_parked(&mut sync.goals);
            match result {
                LastParkedResult::ParkSelf => {
                    should_wait = true;
                }
                LastParkedResult::WakeSelf => {
                    // Continue without waiting.
                }
                LastParkedResult::Wake(n) => {
                    // We are still holding `sync`, so we must not call `notify_work_available`
                    // (which would try to lock it again and deadlock).
                    if wake_all_override() {
                        self.notify_work_available_while_locked(&sync, true);
                    } else {
                        self.wake_n_while_locked(&sync, n);
                    }
                }
                LastParkedResult::WakeAll => {
                    // We are still holding `sync`, so we must not call `notify_work_available`
                    // (which would try to lock it again and deadlock).
                    self.notify_work_available_while_locked(&sync, true);
                }
            }
        } else {
            should_wait = true;
        }

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
        // The actual condition for this wait is:
        //
        // 1.  any work packet is available, or
        // 2.  a goal (such as doing GC) is requested
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
        // Condition (2), i.e. goals added to `sync.goals`, is guarded by the monitor `sync`.
        // When a mutator adds a goal via `WorkerMonitor::make_request`, it will notify a
        // worker; and the last parked worker always checks it before waiting.  So this
        // condition will not be set without any worker noticing.
        //
        // Note that generational barriers may add `ProcessModBuf` work packets when not in GC.
        // This is benign because those work packets are not executed immediately, and are
        // guaranteed to be executed in the next GC.
        //
        // Whether this worker is currently active is a third condition this function waits on.
        // An inactive worker waits on `active_worker_number_changed` instead of
        // `workers_have_anything_to_do`, and both are only ever notified while holding
        // `self.sync` (see `notify_work_available_while_locked`).  Because a worker holds
        // `self.sync` continuously from the point it decides to wait until the point it
        // actually blocks inside `Condvar::wait`, a notification can never be delivered to a
        // worker that has not started waiting yet: the notifier cannot acquire `self.sync`
        // (and therefore cannot notify) until the worker has released it by starting to wait.
        //
        // Notes on spurious wake-up:
        //
        // 1.  The condition variable `workers_have_anything_to_do` is guarded by `self.sync`.
        //     Because the last parked worker is holding the mutex `self.sync` when executing
        //     `on_last_parked`, no workers can unpark (even if they spuriously wake up) during
        //     `on_last_parked` because they cannot re-acquire the mutex `self.sync`.
        //
        // 2.  Workers may spuriously wake up and unpark when `on_last_parked` is not being
        //     executed (including the case when the last parked worker is waiting here, too).
        //     If one or more GC workers spuriously wake up, they will check for work packets,
        //     and park again if not available.  The last parked worker will ensure the two
        //     conditions listed above are both false before blocking.  If either condition is
        //     true, the last parked worker will take action.
        //
        // `should_wait` is `false` only when this worker is the last parked worker and
        // `on_last_parked` returned `WakeSelf` or `WakeAll`, meaning work is already known to be
        // available and this worker must proceed without waiting here.  Note that `should_wait`
        // says nothing about whether this worker is currently active: a worker that is not the
        // last parked one (`should_wait == true`) can still be inactive, e.g. if it was
        // deactivated by `set_active_workers` before it got here.  Such a worker must not wait on
        // `workers_have_anything_to_do` (it must not consume work packets while inactive), so it
        // skips straight to the `while` loop below instead.
        // `sleeping` is incremented while `sync` is still held, before `wait` releases it. That is
        // what lets `notify_work_available` treat a zero reading as "no worker is parked yet".
        if should_wait && self.is_worker_active(ordinal) {
            self.sleeping.fetch_add(1, Ordering::SeqCst);
            sync = self.workers_have_anything_to_do.wait(sync).unwrap();
            self.sleeping.fetch_sub(1, Ordering::SeqCst);
        }

        // The worker may be inactive already (see above), or it may become inactive while
        // waiting above, since its active/inactive status can change at any time `self.sync` is
        // not held.  Keep waiting on `active_worker_number_changed` -- which is notified whenever
        // the active worker count changes -- until this worker is (re)activated.
        while !self.is_worker_active(ordinal) {
            sync.inactive_waiters += 1;
            self.sleeping.fetch_add(1, Ordering::SeqCst);
            sync = self.active_worker_number_changed.wait(sync).unwrap();
            self.sleeping.fetch_sub(1, Ordering::SeqCst);
            sync.inactive_waiters -= 1;
        }

        // Unpark this worker.
        sync.parker.dec_parked_workers();
        trace!(
            "Worker {} unparked.  parked/total: {}/{}.",
            ordinal,
            sync.parker.parked_workers,
            sync.parker.worker_count,
        );

        // If the current goal is an exit goal, the worker thread should exit.
        if matches!(
            sync.goals.current(),
            Some(WorkerGoal::Shutdown | WorkerGoal::StopForFork)
        ) {
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

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    use super::WorkerMonitor;

    /// Test if the `WorkerMonitor::park_and_wait` method calls the `on_last_parked` callback
    /// properly.
    #[test]
    fn test_last_worker_park_wake_all() {
        let number_threads = 4;
        let worker_monitor = Arc::new(WorkerMonitor::new(number_threads));
        let on_last_parked_called = AtomicUsize::new(0);
        let should_unpark = AtomicBool::new(false);

        std::thread::scope(|scope| {
            for ordinal in 0..number_threads {
                let worker_monitor = worker_monitor.clone();
                let on_last_parked_called = &on_last_parked_called;
                let should_unpark = &should_unpark;
                scope.spawn(move || {
                    // This emulates the use pattern in the scheduler, i.e. checking the condition
                    // ("Is there any work packets available") without holding a mutex.
                    while !should_unpark.load(Ordering::SeqCst) {
                        println!("Thread {} parking...", ordinal);
                        worker_monitor
                            .park_and_wait(ordinal, |_goals| {
                                println!("Thread {} is the last thread parked.", ordinal);
                                on_last_parked_called.fetch_add(1, Ordering::SeqCst);
                                should_unpark.store(true, Ordering::SeqCst);
                                super::LastParkedResult::WakeAll
                            })
                            .unwrap();
                        println!("Thread {} unparked.", ordinal);
                    }
                });
            }
        });

        // `on_last_parked` should only be called once.
        assert_eq!(on_last_parked_called.load(Ordering::SeqCst), 1);
    }

    /// Like `test_last_worker_park_wake_all`, but only wake up the last parked worker when it
    /// parked.
    #[test]
    fn test_last_worker_park_wake_self() {
        let number_threads = 4;
        let worker_monitor = Arc::new(WorkerMonitor::new(number_threads));
        let on_last_parked_called = AtomicUsize::new(0);
        let threads_running = AtomicUsize::new(0);
        let should_unpark = AtomicBool::new(false);

        std::thread::scope(|scope| {
            for ordinal in 0..number_threads {
                let worker_monitor = worker_monitor.clone();
                let on_last_parked_called = &on_last_parked_called;
                let threads_running = &threads_running;
                let should_unpark = &should_unpark;
                scope.spawn(move || {
                    let mut i_am_the_last_parked_worker = false;
                    // Record the number of threads entering the following `while` loop.
                    threads_running.fetch_add(1, Ordering::SeqCst);
                    while !should_unpark.load(Ordering::SeqCst) {
                        println!("Thread {} parking...", ordinal);
                        worker_monitor
                            .park_and_wait(ordinal, |_goals| {
                                println!("Thread {} is the last thread parked.", ordinal);
                                on_last_parked_called.fetch_add(1, Ordering::SeqCst);
                                should_unpark.store(true, Ordering::SeqCst);
                                i_am_the_last_parked_worker = true;
                                super::LastParkedResult::WakeSelf
                            })
                            .unwrap();
                        println!("Thread {} unparked.", ordinal);
                    }
                    threads_running.fetch_sub(1, Ordering::SeqCst);

                    if i_am_the_last_parked_worker {
                        println!("The last parked worker woke up");
                        // Only the current worker should wake and leave the `while` loop above.
                        assert_eq!(threads_running.load(Ordering::SeqCst), number_threads - 1);
                        should_unpark.store(true, Ordering::SeqCst);
                        worker_monitor.notify_work_available(true);
                    }
                });
            }
        });

        // `on_last_parked` should only be called once.
        assert_eq!(on_last_parked_called.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_only_selected_workers_unpark() {
        let number_threads = 4;
        let concurrent_threads = 2;
        let worker_monitor = Arc::new(WorkerMonitor::new(number_threads));
        worker_monitor.set_active_workers(concurrent_threads);
        let first_wave_unparked = AtomicUsize::new(0);
        let release_everyone = AtomicBool::new(false);
        let notifier_ran = AtomicBool::new(false);

        std::thread::scope(|scope| {
            for ordinal in 0..number_threads {
                let worker_monitor = worker_monitor.clone();
                let first_wave_unparked = &first_wave_unparked;
                let release_everyone = &release_everyone;
                let notifier_ran = &notifier_ran;
                scope.spawn(move || {
                    worker_monitor
                        .park_and_wait(ordinal, |_goals| super::LastParkedResult::WakeAll)
                        .unwrap();

                    if !release_everyone.load(Ordering::SeqCst) {
                        first_wave_unparked.fetch_add(1, Ordering::SeqCst);
                    }

                    if ordinal < concurrent_threads {
                        while first_wave_unparked.load(Ordering::SeqCst) < concurrent_threads {
                            std::thread::yield_now();
                        }
                        if !notifier_ran.swap(true, Ordering::SeqCst) {
                            release_everyone.store(true, Ordering::SeqCst);
                            worker_monitor.set_active_workers(number_threads);
                            worker_monitor.notify_work_available(true);
                        }
                    }
                });
            }
        });

        assert_eq!(
            first_wave_unparked.load(Ordering::SeqCst),
            concurrent_threads
        );
    }
}
