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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Represents the ID of a GC worker thread.
pub type ThreadId = usize;

thread_local! {
    /// Current worker's ordinal
    static WORKER_ORDINAL: Atomic<Option<ThreadId>> = Atomic::new(None);
}

/// Get current worker ordinal. Return `None` if the current thread is not a worker.
pub fn current_worker_ordinal() -> Option<ThreadId> {
    WORKER_ORDINAL.with(|x| x.load(Ordering::Relaxed))
}

/// The part shared between a GCWorker and the scheduler.
/// This structure is used for communication, e.g. adding new work packets.
pub struct GCWorkerShared<VM: VMBinding> {
    /// Worker-local statistics data.
    stat: AtomicRefCell<WorkerLocalStat<VM>>,
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
            designated_work: ArrayQueue::new(16),
            stealer,
        }
    }
}

pub(crate) struct WorkerMonitorSync {
    /// This flag is set to true when all workers have parked.
    /// No workers can unpark when this is set.
    /// This flag is cleared if a work packet is added to an open bucket,
    /// or a new bucket is opened.
    /// The main purpose of this flag is handling spurious wake-ups so that workers will not
    /// attempt to inspect bucket states while the coordinator is opening/closing buckets.
    pub group_sleep: bool,
}

/// Used to synchronize mutually exclusive operations between workers and controller,
/// and also waking up workers when more work packets are available.
/// NOTE: The `sync` and `cond` fields are public in order to support the complex control structure
/// in `GCWorkScheduler::poll_slow`.
pub(crate) struct WorkerMonitor {
    pub sync: Mutex<WorkerMonitorSync>,
    pub cond: Condvar,
}

impl Default for WorkerMonitor {
    fn default() -> Self {
        Self {
            sync: Mutex::new(WorkerMonitorSync { group_sleep: false }),
            cond: Default::default(),
        }
    }
}

impl WorkerMonitor {
    pub(crate) fn notify_work_available(&self, all: bool) {
        let mut sync = self.sync.lock().unwrap();
        sync.group_sleep = false;
        if all {
            self.cond.notify_all();
        } else {
            self.cond.notify_one();
        }
    }

    pub fn is_group_sleeping(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.group_sleep
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
    /// The sending end of the channel to send message to the controller thread.
    pub(crate) sender: controller::channel::Sender<VM>,
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
        sender: controller::channel::Sender<VM>,
        shared: Arc<GCWorkerShared<VM>>,
        local_work_buffer: deque::Worker<Box<dyn GCWork<VM>>>,
    ) -> Self {
        Self {
            tls: VMWorkerThread(VMThread::UNINITIALIZED),
            ordinal,
            // We will set this later
            copy: GCWorkerCopyContext::new_non_copy(),
            sender,
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

    pub fn is_coordinator(&self) -> bool {
        self.is_coordinator
    }

    pub fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    pub fn get_copy_context_mut(&mut self) -> &mut GCWorkerCopyContext<VM> {
        &mut self.copy
    }

    pub fn do_work(&'static mut self, mut work: impl GCWork<VM>) {
        work.do_work(self, self.mmtk);
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

    pub fn do_boxed_work(&'static mut self, mut work: Box<dyn GCWork<VM>>) {
        work.do_work(self, self.mmtk);
    }

    /// Entry of the worker thread. Resolve thread affinity, if it has been specified by the user.
    /// Each worker will keep polling and executing work packets in a loop.
    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        WORKER_ORDINAL.with(|x| x.store(Some(self.ordinal), Ordering::SeqCst));
        self.scheduler.resolve_affinity(self.ordinal);
        self.tls = tls;
        self.copy = crate::plan::create_gc_worker_context(tls, mmtk);
        loop {
            let mut work = self.poll();
            work.do_work_with_stat(self, mmtk);
        }
    }
}

/// A worker group to manage all the GC workers (except the coordinator worker).
pub(crate) struct WorkerGroup<VM: VMBinding> {
    /// Shared worker data
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
    parked_workers: AtomicUsize,
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
            parked_workers: Default::default(),
            unspawned_local_work_queues: Mutex::new(unspawned_local_work_queues),
        })
    }

    /// Spawn all the worker threads
    pub fn spawn(
        &self,
        mmtk: &'static MMTK<VM>,
        sender: controller::channel::Sender<VM>,
        tls: VMThread,
    ) {
        let mut unspawned_local_work_queues = self.unspawned_local_work_queues.lock().unwrap();
        // Spawn each worker thread.
        for (ordinal, shared) in self.workers_shared.iter().enumerate() {
            let worker = Box::new(GCWorker::new(
                mmtk,
                ordinal,
                mmtk.scheduler.clone(),
                false,
                sender.clone(),
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

    /// Increase the packed-workers counter.
    /// Called before a worker is parked.
    ///
    /// Return true if all the workers are parked.
    pub fn inc_parked_workers(&self) -> bool {
        let old = self.parked_workers.fetch_add(1, Ordering::SeqCst);
        debug_assert!(old < self.worker_count());
        old + 1 == self.worker_count()
    }

    /// Decrease the packed-workers counter.
    /// Called after a worker is resumed from the parked state.
    pub fn dec_parked_workers(&self) {
        let old = self.parked_workers.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(old <= self.worker_count());
    }

    /// Return true if there're any pending designated work
    pub fn has_designated_work(&self) -> bool {
        self.workers_shared
            .iter()
            .any(|w| !w.designated_work.is_empty())
    }
}

/// This ensures the worker always decrements the parked worker count on all control flow paths.
pub(crate) struct ParkingGuard<'a, VM: VMBinding> {
    worker_group: &'a WorkerGroup<VM>,
    all_parked: bool,
}

impl<'a, VM: VMBinding> ParkingGuard<'a, VM> {
    pub fn new(worker_group: &'a WorkerGroup<VM>) -> Self {
        let all_parked = worker_group.inc_parked_workers();
        ParkingGuard {
            worker_group,
            all_parked,
        }
    }

    pub fn all_parked(&self) -> bool {
        self.all_parked
    }
}

impl<'a, VM: VMBinding> Drop for ParkingGuard<'a, VM> {
    fn drop(&mut self) {
        self.worker_group.dec_parked_workers();
    }
}
