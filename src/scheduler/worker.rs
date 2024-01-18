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

/// The struct has one instance per worker, but is shared between workers via the scheduler
/// instance.  This structure is used for communication between workers, e.g. adding designated
/// work packets, stealing work packets from other workers, and collecting per-worker statistics.
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

/// A special error type that indicate a worker should exit.
/// This may happen if the VM needs to fork and asks workers to exit.
pub(crate) struct WorkerShouldExit;

/// The result type of `GCWorker::pool`.
/// Too many functions return `Option<Box<dyn GCWork<VM>>>`.  In most cases, when `None` is
/// returned, the caller should try getting work packets from another place.  To avoid confusion,
/// we use `Err(WorkerShouldExit)` to clearly indicate that the worker should exit immediately.
pub(crate) type PollResult<VM> = Result<Box<dyn GCWork<VM>>, WorkerShouldExit>;

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
    fn poll(&mut self) -> PollResult<VM> {
        if let Some(work) = self.shared.designated_work.pop() {
            return Ok(work);
        }

        if let Some(work) = self.local_work_buffer.pop() {
            return Ok(work);
        }

        self.scheduler().poll(self)
    }

    /// Entry of the worker thread. Resolve thread affinity, if it has been specified by the user.
    /// Each worker will keep polling and executing work packets in a loop.
    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        probe!(mmtk, gcworker_run);
        warn!("Worker {} started.", self.ordinal);
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
            let Ok(mut work) = self.poll() else {
                // The worker is asked to exit.  Break from the loop.
                break;
            };
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
        probe!(mmtk, gcworker_exit);
    }
}

enum WorkerCreationState<VM: VMBinding> {
    NotCreated {
        /// The local work queues for to-be-created workers.
        unspawned_local_work_queues: Vec<deque::Worker<Box<dyn GCWork<VM>>>>,
    },
    Created {
        /// `Worker` instances for worker threads that have not been created yet, or worker thread that
        /// are suspended (e.g. for forking).
        suspended_workers: Vec<Box<GCWorker<VM>>>,
    },
}

impl<VM: VMBinding> WorkerCreationState<VM> {
    pub fn is_created(&self) -> bool {
        matches!(self, WorkerCreationState::Created { .. })
    }
}

/// A worker group to manage all the GC workers.
pub(crate) struct WorkerGroup<VM: VMBinding> {
    /// Shared worker data
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
    /// The stateful part
    state: Mutex<WorkerCreationState<VM>>,
    /// The condition of "all workers exited", i.e. the number of suspended workers is equal to the
    /// number of workers.
    cond_all_workers_exited: Condvar,
}

/// We have to persuade Rust that `WorkerGroup` is safe to share because it thinks one worker can
/// refer to another worker via the path "worker -> scheduler -> worker_group -> suspended_workers
/// -> worker" which it thinks is cyclic reference and unsafe.
unsafe impl<VM: VMBinding> Sync for WorkerGroup<VM> {}

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
            state: Mutex::new(WorkerCreationState::NotCreated {
                unspawned_local_work_queues,
            }),
            cond_all_workers_exited: Default::default(),
        })
    }

    /// Create `GCWorker` instances, but do not create threads, yet.
    fn create_worker_structs(&self, state: &mut WorkerCreationState<VM>, mmtk: &'static MMTK<VM>) {
        let WorkerCreationState::NotCreated { unspawned_local_work_queues } = state else {
            panic!("GCWorker structs have already been created");
        };

        assert_eq!(self.workers_shared.len(), unspawned_local_work_queues.len());
        let mut suspended_workers = Vec::with_capacity(self.workers_shared.len());

        // Spawn each worker thread.
        for (ordinal, shared) in self.workers_shared.iter().enumerate() {
            let worker = Box::new(GCWorker::new(
                mmtk,
                ordinal,
                mmtk.scheduler.clone(),
                shared.clone(),
                unspawned_local_work_queues.pop().unwrap(),
            ));
            suspended_workers.push(worker);
        }

        *state = WorkerCreationState::Created { suspended_workers };
    }

    /// Spawn all the worker threads
    pub fn spawn(&self, mmtk: &'static MMTK<VM>, tls: VMThread) {
        let mut guard = self.state.lock().unwrap();

        // Create worker structs lazily.  If we are spawning workers after fork, worker structs
        // will have been created already, so we can skip the creation.
        if !guard.is_created() {
            self.create_worker_structs(&mut *guard, mmtk);
        }

        let WorkerCreationState::Created { ref mut suspended_workers } = *guard else {
            panic!("GCWorker structs have not been created, yet.");
        };

        // Drain the queue.  We transfer the ownership of each `GCWorker` instance to a GC thread.
        for worker in suspended_workers.drain(..) {
            VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Worker(worker));
        }
    }

    /// Surrender the `GCWorker` struct when a GC worker exits.
    pub fn surrender_gc_worker(&self, worker: Box<GCWorker<VM>>) {
        let mut state = self.state.lock().unwrap();
        let WorkerCreationState::Created { ref mut suspended_workers } = *state else {
            panic!("GCWorker structs have not been created, yet.");
        };
        let ordinal = worker.ordinal;
        suspended_workers.push(worker);
        info!(
            "Worker {} surrendered. ({}/{})",
            ordinal,
            suspended_workers.len(),
            self.worker_count()
        );
        if suspended_workers.len() == self.worker_count() {
            info!("All {} workers surrendered.", self.worker_count());
            self.cond_all_workers_exited.notify_all();
        }
    }

    /// Wait until all workers exited.
    pub fn wait_until_worker_exited(&self) {
        let guard = self.state.lock().unwrap();
        let _guard = self.cond_all_workers_exited.wait_while(guard, |state| {
            let WorkerCreationState::Created { ref suspended_workers } = *state else {
                panic!("GCWorker structs have not been created, yet.");
            };
            suspended_workers.len() != self.worker_count()
        });
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
