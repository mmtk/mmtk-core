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
use std::sync::{Arc, Condvar, Mutex};

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
    ///
    /// Note: Currently, designated work cannot be added from the GC controller thread, or
    /// there will be synchronization problems.  If it is necessary to do so, we need to
    /// update the code in `GCWorkScheduler::poll_slow` for proper synchornization.
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

/// Used to synchronize mutually exclusive operations between workers and controller,
/// and also waking up workers when more work packets are available.
pub(crate) struct WorkerMonitor {
    /// The synchronized part.
    sync: Mutex<WorkerMonitorSync>,
    /// This is notified when new work is made available for the workers.
    /// Particularly, it is notified when
    /// -   `sync.worker_group_state` is transitioned to `Working` because
    ///     -   some workers still have designated work, or
    ///     -   some sentinel work packets are added to their drained buckets, or
    ///     -   some work buckets are opened, or
    /// -   any work packet is added to any open bucket.
    /// Workers wait on this condvar.
    work_available: Condvar,
    /// This is notified when all workers parked.
    /// The coordinator waits on this condvar.
    all_workers_parked: Condvar,
}

/// The state of the worker group.
///
/// The worker group alternates between the `Sleeping` and the `Working` state.  Workers are
/// allowed to execute work packets in the `Working` state.  However, once workers entered the
/// `Sleeping` state, they will not be allowed to packets from buckets until the coordinator
/// explicitly transitions the state back to `Working` after it found more work for workers to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WorkerGroupState {
    /// In this state, the coordinator can open new buckets and close buckets,
    /// but workers cannot execute any packets or get any work packets from any buckets.
    /// Workers cannot unpark in this state.
    Sleeping,
    /// In this state, workers can get work packets from open buckets,
    /// but no buckets can be opened or closed.
    Working,
}

/// The synchronized part of `WorkerMonitor`.
pub(crate) struct WorkerMonitorSync {
    /// The total number of workers.
    worker_count: usize,
    /// Number of parked workers.
    parked_workers: usize,
    /// The worker group state.
    worker_group_state: WorkerGroupState,
}

impl WorkerMonitor {
    pub fn new(worker_count: usize) -> Self {
        Self {
            sync: Mutex::new(WorkerMonitorSync {
                worker_count,
                parked_workers: 0,
                worker_group_state: WorkerGroupState::Sleeping,
            }),
            work_available: Default::default(),
            all_workers_parked: Default::default(),
        }
    }

    /// Wake up workers when more work packets are made available for workers.
    /// This function is called when adding work packets to buckets.
    /// This function doesn't change the `work_group_state` variable.
    /// If workers are in the `Sleeping` state, use `resume_and_wait` to resume workers.
    pub fn notify_work_available(&self, all: bool) {
        let sync = self.sync.lock().unwrap();

        // Don't notify workers if we are adding packets when workers are sleeping.
        // This could happen when we add `ScheduleCollection` or schedule sentinels.
        if sync.worker_group_state == WorkerGroupState::Sleeping {
            return;
        }

        if all {
            self.work_available.notify_all();
        } else {
            self.work_available.notify_one();
        }
    }

    /// Wake up workers and wait until they transition to `Sleeping` state again.
    /// This is called by the coordinator.
    /// If `all` is true, notify all workers; otherwise only notify one worker.
    pub fn resume_and_wait(&self, all: bool) {
        let mut sync = self.sync.lock().unwrap();
        sync.worker_group_state = WorkerGroupState::Working;
        if all {
            self.work_available.notify_all();
        } else {
            self.work_available.notify_one();
        }
        let _sync = self
            .all_workers_parked
            .wait_while(sync, |sync| {
                sync.worker_group_state == WorkerGroupState::Working
            })
            .unwrap();
    }

    /// Test if the worker group is in the `Sleeping` state.
    pub fn debug_is_sleeping(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.worker_group_state == WorkerGroupState::Sleeping
    }

    /// Park until more work is available.
    /// The argument `worker` indicates this function can only be called by workers.
    pub fn park_and_wait<VM: VMBinding>(&self, worker: &GCWorker<VM>) {
        let mut sync = self.sync.lock().unwrap();

        // Park this worker
        let all_parked = sync.inc_parked_workers();
        trace!("Worker {} parked.", worker.ordinal);

        if all_parked {
            // If all workers are parked, enter "Sleeping" state and notify controller.
            sync.worker_group_state = WorkerGroupState::Sleeping;
            debug!(
                "Worker {} notifies the coordinator that all workerer parked.",
                worker.ordinal
            );
            self.all_workers_parked.notify_one();
        } else {
            // Otherwise wait until notified.
            // Note: The condition for this `cond.wait` is "more work is available".
            // If this worker spuriously wakes up, then in the next loop iteration, the
            // `poll_schedulable_work` invocation above will fail, and the worker will reach
            // here and wait again.
            sync = self.work_available.wait(sync).unwrap();
        }

        // If we are in the `Sleeping` state, wait until leaving that state.
        sync = self
            .work_available
            .wait_while(sync, |sync| {
                sync.worker_group_state == WorkerGroupState::Sleeping
            })
            .unwrap();

        // Unpark this worker.
        sync.dec_parked_workers();
        trace!("Worker {} unparked.", worker.ordinal);
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
/// The GC controller also has an embedded `GCWorker` because it may also execute work packets.
pub struct GCWorker<VM: VMBinding> {
    /// The VM-specific thread-local state of the GC thread.
    pub tls: VMWorkerThread,
    /// The ordinal of the worker, numbered from 0 to the number of workers minus one. The ordinal
    /// is usize::MAX if it is the embedded worker of the GC controller thread.
    pub ordinal: ThreadId,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The copy context, used to implement copying GC.
    copy: GCWorkerCopyContext<VM>,
    /// The reference to the MMTk instance.
    pub mmtk: &'static MMTK<VM>,
    /// True if this struct is the embedded GCWorker of the controller thread.
    /// False if this struct belongs to a standalone GCWorker thread.
    is_coordinator: bool,
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
        is_coordinator: bool,
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
            is_coordinator,
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

    /// Is this worker a coordinator or a normal GC worker?
    pub fn is_coordinator(&self) -> bool {
        self.is_coordinator
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

/// A worker group to manage all the GC workers (except the coordinator worker).
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
                false,
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
