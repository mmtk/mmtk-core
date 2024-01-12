use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::copy::GCWorkerCopyContext;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, GCThreadContext, VMBinding};
use atomic::Atomic;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use crossbeam::deque::{self, Stealer};
use crossbeam::queue::ArrayQueue;
#[cfg(feature = "count_live_bytes_in_gc")]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// Represents the ID of a GC worker thread.
pub type ThreadId = usize;

thread_local! {
    /// Current worker's ordinal
    static WORKER_ORDINAL: Atomic<ThreadId> = Atomic::new(ThreadId::MAX);
}

/// Get current worker ordinal. Return `None` if the current thread is not a worker.
pub fn current_worker_ordinal() -> ThreadId {
    let ordinal = WORKER_ORDINAL.with(|x| x.load(Ordering::Relaxed));
    debug_assert_ne!(
        ordinal,
        ThreadId::MAX,
        "Thread-local variable WORKER_ORDINAL not set yet."
    );
    ordinal
}

/// The part shared between a GCWorker and the scheduler.
/// This structure is used for communication, e.g. adding new work packets.
pub struct GCWorkerShared<VM: VMBinding> {
    /// Worker-local statistics data.
    stat: AtomicRefCell<WorkerLocalStat<VM>>,
    /// Accumulated bytes for live objects in this GC. When each worker scans
    /// objects, we increase the live bytes. We get this value from each worker
    /// at the end of a GC, and reset this counter.
    #[cfg(feature = "count_live_bytes_in_gc")]
    live_bytes: AtomicUsize,
    /// A queue of GCWork that can only be processed by the owned thread.
    pub designated_work: ArrayQueue<Box<dyn GCWork<VM>>>,
    /// Handle for stealing packets from the current worker
    pub stealer: Option<Stealer<Box<dyn GCWork<VM>>>>,
}

impl<VM: VMBinding> GCWorkerShared<VM> {
    pub fn new(stealer: Option<Stealer<Box<dyn GCWork<VM>>>>) -> Self {
        Self {
            stat: Default::default(),
            #[cfg(feature = "count_live_bytes_in_gc")]
            live_bytes: AtomicUsize::new(0),
            designated_work: ArrayQueue::new(16),
            stealer,
        }
    }

    #[cfg(feature = "count_live_bytes_in_gc")]
    pub(crate) fn increase_live_bytes(&self, bytes: usize) {
        self.live_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    #[cfg(feature = "count_live_bytes_in_gc")]
    pub(crate) fn get_and_clear_live_bytes(&self) -> usize {
        self.live_bytes.swap(0, Ordering::SeqCst)
    }
}

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

/// A data structure for synchronizing workers.
///
/// -   It allows workers to park and unpark.  It keeps track of the number of workers parked, and
///     allows the last parked worker to perform operations that can only be performed when all
///     workers have parked.
/// -   It allows mutators to notify workers to schedule a GC.
pub(crate) struct WorkerMonitor {
    /// The synchronized part.
    sync: Mutex<WorkerMonitorSync>,
    /// Notified if workers have anything to do.  That include any work packets available, and any
    /// field in `sync.goals.requests` set to true.
    have_anything_to_do: Condvar,
}

/// The synchronized part of `WorkerMonitor`.
pub(crate) struct WorkerMonitorSync {
    /// The total number of workers.
    worker_count: usize,
    /// Number of parked workers.
    parked_workers: usize,
    /// Current and requested goals.
    goals: WorkerGoals,
}

#[derive(Default)]
pub(crate) struct WorkerGoals {
    /// What are the workers doing now?
    pub(crate) current: Option<WorkerGoal>,
    /// Requests received from mutators.
    pub(crate) requests: WorkerRequests,
}

/// The thing workers are currently doing.  This affects several things, such as what the last
/// parked worker will do, and whether workers will stop themselves.
pub(crate) enum WorkerGoal {
    Gc {
        start_time: std::time::Instant,
    },
    #[allow(unused)] // TODO: Implement forking support later.
    StopForFork,
}

/// Reqeusts received from mutators.  Workers respond to those requests when they do not have a
/// current goal.  Multiple things can be requested at the same time, and workers respond to the
/// thing with the highest priority.
///
/// The fields of this structs are ordered with decreasing priority.
#[derive(Default)] // All fields should be false by default.
pub(crate) struct WorkerRequests {
    /// The VM needs to fork.  Workers should save their contexts and exit.
    pub(crate) stop_for_fork: bool,
    /// GC is requested.  Workers should schedule a GC.
    pub(crate) gc: bool,
}

impl WorkerMonitor {
    pub fn new(worker_count: usize) -> Self {
        Self {
            sync: Mutex::new(WorkerMonitorSync {
                worker_count,
                parked_workers: 0,
                goals: Default::default(),
            }),
            have_anything_to_do: Default::default(),
        }
    }

    /// Request a GC worker to schedule the next GC.
    /// Callable from mutator threads.
    pub fn request_schedule_collection(&self) {
        let mut guard = self.sync.lock().unwrap();
        if !guard.goals.requests.gc {
            guard.goals.requests.gc = true;
            self.notify_work_available_inner(false, &mut guard);
        }
    }

    /// Wake up workers when more work packets are made available for workers,
    /// or a mutator has requested the GC workers to schedule a GC.
    pub fn notify_work_available(&self, all: bool) {
        let mut guard = self.sync.lock().unwrap();
        self.notify_work_available_inner(all, &mut guard);
    }

    /// Like `notify_work_available` but the current thread must have already acquired the
    /// mutex of `WorkerMonitorSync`.
    fn notify_work_available_inner(&self, all: bool, _guard: &mut MutexGuard<WorkerMonitorSync>) {
        if all {
            self.have_anything_to_do.notify_all();
        } else {
            self.have_anything_to_do.notify_one();
        }
    }

    /// Park a worker and wait on the CondVar `work_available`.
    ///
    /// If it is the last worker parked, `on_last_parked` will be called.
    /// The argument of `on_last_parked` is true if `sync.gc_requested` is `true`.
    /// The return value of `on_last_parked` will determine whether this worker and other workers
    /// will wake up or block waiting.
    pub fn park_and_wait<VM, F>(&self, worker: &GCWorker<VM>, on_last_parked: F)
    where
        VM: VMBinding,
        F: FnOnce(&mut WorkerGoals) -> LastParkedResult,
    {
        let mut sync = self.sync.lock().unwrap();

        // Park this worker
        let all_parked = sync.inc_parked_workers();
        trace!(
            "Worker {} parked.  parked/total: {}/{}.  All parked: {}",
            worker.ordinal,
            sync.parked_workers,
            sync.worker_count,
            all_parked
        );

        let mut should_wait = false;

        if all_parked {
            debug!("Worker {} is the last worker parked.", worker.ordinal);
            let result = on_last_parked(&mut sync.goals);
            match result {
                LastParkedResult::ParkSelf => {
                    should_wait = true;
                }
                LastParkedResult::WakeSelf => {
                    // Continue without waiting.
                }
                LastParkedResult::WakeAll => {
                    self.notify_work_available_inner(true, &mut sync);
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
            sync = self.have_anything_to_do.wait(sync).unwrap();
        }

        // Unpark this worker.
        sync.dec_parked_workers();
        trace!(
            "Worker {} unparked.  parked/total: {}/{}.",
            worker.ordinal,
            sync.parked_workers,
            sync.worker_count,
        );
    }
}

impl WorkerMonitorSync {
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

/// A GC worker.  This part is privately owned by a worker thread.
pub struct GCWorker<VM: VMBinding> {
    /// The VM-specific thread-local state of the GC thread.
    pub tls: VMWorkerThread,
    /// The ordinal of the worker, numbered from 0 to the number of workers minus one.
    pub ordinal: ThreadId,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The copy context, used to implement copying GC.
    copy: GCWorkerCopyContext<VM>,
    /// The reference to the MMTk instance.
    pub mmtk: &'static MMTK<VM>,
    /// Reference to the shared part of the GC worker.  It is used for synchronization.
    pub shared: Arc<GCWorkerShared<VM>>,
    /// Local work packet queue.
    pub local_work_buffer: deque::Worker<Box<dyn GCWork<VM>>>,
}

unsafe impl<VM: VMBinding> Sync for GCWorkerShared<VM> {}
unsafe impl<VM: VMBinding> Send for GCWorkerShared<VM> {}

// Error message for borrowing `GCWorkerShared::stat`.
const STAT_BORROWED_MSG: &str = "GCWorkerShared.stat is already borrowed.  This may happen if \
    the mutator calls harness_begin or harness_end while the GC is running.";

impl<VM: VMBinding> GCWorkerShared<VM> {
    pub fn borrow_stat(&self) -> AtomicRef<WorkerLocalStat<VM>> {
        self.stat.try_borrow().expect(STAT_BORROWED_MSG)
    }

    pub fn borrow_stat_mut(&self) -> AtomicRefMut<WorkerLocalStat<VM>> {
        self.stat.try_borrow_mut().expect(STAT_BORROWED_MSG)
    }
}

impl<VM: VMBinding> GCWorker<VM> {
    pub(crate) fn new(
        mmtk: &'static MMTK<VM>,
        ordinal: ThreadId,
        scheduler: Arc<GCWorkScheduler<VM>>,
        shared: Arc<GCWorkerShared<VM>>,
        local_work_buffer: deque::Worker<Box<dyn GCWork<VM>>>,
    ) -> Self {
        Self {
            tls: VMWorkerThread(VMThread::UNINITIALIZED),
            ordinal,
            // We will set this later
            copy: GCWorkerCopyContext::new_non_copy(),
            scheduler,
            mmtk,
            shared,
            local_work_buffer,
        }
    }

    const LOCALLY_CACHED_WORK_PACKETS: usize = 16;

    /// Add a work packet to the work queue and mark it with a higher priority.
    /// If the bucket is activated, the packet will be pushed to the local queue, otherwise it will be
    /// pushed to the global bucket with a higher priority.
    pub fn add_work_prioritized(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || self.local_work_buffer.len() >= Self::LOCALLY_CACHED_WORK_PACKETS
        {
            self.scheduler.work_buckets[bucket].add_prioritized(Box::new(work));
            return;
        }
        self.local_work_buffer.push(Box::new(work));
    }

    /// Add a work packet to the work queue.
    /// If the bucket is activated, the packet will be pushed to the local queue, otherwise it will be
    /// pushed to the global bucket.
    pub fn add_work(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || self.local_work_buffer.len() >= Self::LOCALLY_CACHED_WORK_PACKETS
        {
            self.scheduler.work_buckets[bucket].add(work);
            return;
        }
        self.local_work_buffer.push(Box::new(work));
    }

    /// Get the scheduler. There is only one scheduler per MMTk instance.
    pub fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    /// Get a mutable reference of the copy context for this worker.
    pub fn get_copy_context_mut(&mut self) -> &mut GCWorkerCopyContext<VM> {
        &mut self.copy
    }

    /// Poll a ready-to-execute work packet in the following order:
    ///
    /// 1. Any packet that should be processed only by this worker.
    /// 2. Poll from the local work queue.
    /// 3. Poll from activated global work-buckets
    /// 4. Steal from other workers
    fn poll(&self) -> Box<dyn GCWork<VM>> {
        self.shared
            .designated_work
            .pop()
            .or_else(|| self.local_work_buffer.pop())
            .unwrap_or_else(|| self.scheduler().poll(self))
    }

    /// Entry of the worker thread. Resolve thread affinity, if it has been specified by the user.
    /// Each worker will keep polling and executing work packets in a loop.
    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        probe!(mmtk, gcworker_run);
        WORKER_ORDINAL.with(|x| x.store(self.ordinal, Ordering::SeqCst));
        self.scheduler.resolve_affinity(self.ordinal);
        self.tls = tls;
        self.copy = crate::plan::create_gc_worker_context(tls, mmtk);
        loop {
            // Instead of having work_start and work_end tracepoints, we have
            // one tracepoint before polling for more work and one tracepoint
            // before executing the work.
            // This allows measuring the distribution of both the time needed
            // poll work (between work_poll and work), and the time needed to
            // execute work (between work and next work_poll).
            // If we have work_start and work_end, we cannot measure the first
            // poll.
            probe!(mmtk, work_poll);
            let mut work = self.poll();
            // probe! expands to an empty block on unsupported platforms
            #[allow(unused_variables)]
            let typename = work.get_type_name();

            #[cfg(feature = "bpftrace_workaround")]
            // Workaround a problem where bpftrace script cannot see the work packet names,
            // by force loading from the packet name.
            // See the "Known issues" section in `tools/tracing/timeline/README.md`
            std::hint::black_box(unsafe { *(typename.as_ptr()) });

            probe!(mmtk, work, typename.as_ptr(), typename.len());
            work.do_work_with_stat(self, mmtk);
        }
    }
}

/// A worker group to manage all the GC workers.
pub(crate) struct WorkerGroup<VM: VMBinding> {
    /// Shared worker data
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
    unspawned_local_work_queues: Mutex<Vec<deque::Worker<Box<dyn GCWork<VM>>>>>,
}

impl<VM: VMBinding> WorkerGroup<VM> {
    /// Create a WorkerGroup
    pub fn new(num_workers: usize) -> Arc<Self> {
        let unspawned_local_work_queues = (0..num_workers)
            .map(|_| deque::Worker::new_fifo())
            .collect::<Vec<_>>();

        let workers_shared = (0..num_workers)
            .map(|i| {
                Arc::new(GCWorkerShared::<VM>::new(Some(
                    unspawned_local_work_queues[i].stealer(),
                )))
            })
            .collect::<Vec<_>>();

        Arc::new(Self {
            workers_shared,
            unspawned_local_work_queues: Mutex::new(unspawned_local_work_queues),
        })
    }

    /// Spawn all the worker threads
    pub fn spawn(&self, mmtk: &'static MMTK<VM>, tls: VMThread) {
        let mut unspawned_local_work_queues = self.unspawned_local_work_queues.lock().unwrap();
        // Spawn each worker thread.
        for (ordinal, shared) in self.workers_shared.iter().enumerate() {
            let worker = Box::new(GCWorker::new(
                mmtk,
                ordinal,
                mmtk.scheduler.clone(),
                shared.clone(),
                unspawned_local_work_queues.pop().unwrap(),
            ));
            VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Worker(worker));
        }
        debug_assert!(unspawned_local_work_queues.is_empty());
    }

    /// Get the number of workers in the group
    pub fn worker_count(&self) -> usize {
        self.workers_shared.len()
    }

    /// Return true if there're any pending designated work
    pub fn has_designated_work(&self) -> bool {
        self.workers_shared
            .iter()
            .any(|w| !w.designated_work.is_empty())
    }

    #[cfg(feature = "count_live_bytes_in_gc")]
    pub fn get_and_clear_worker_live_bytes(&self) -> usize {
        self.workers_shared
            .iter()
            .map(|w| w.get_and_clear_live_bytes())
            .sum()
    }
}
