use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::copy::GCWorkerCopyContext;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, GCThreadContext, VMBinding};
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use crossbeam::deque::{self, Stealer};
use crossbeam::queue::ArrayQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// The part shared between a GCWorker and the scheduler.
/// This structure is used for communication, e.g. adding new work packets.
pub struct GCWorkerShared<VM: VMBinding> {
    /// Worker-local statistics data.
    stat: AtomicRefCell<WorkerLocalStat<VM>>,
    /// A queue of GCWork that can only be processed by the owned thread.
    pub local_work: ArrayQueue<Box<dyn GCWork<VM>>>,
    /// Local work packet queue.
    pub local_work_buffer: deque::Worker<Box<dyn GCWork<VM>>>,
}

impl<VM: VMBinding> Default for GCWorkerShared<VM> {
    fn default() -> Self {
        Self::new()
    }
}

impl<VM: VMBinding> GCWorkerShared<VM> {
    pub fn new() -> Self {
        Self {
            stat: Default::default(),
            local_work: ArrayQueue::new(16),
            local_work_buffer: deque::Worker::new_fifo(),
        }
    }
}

/// A GC worker.  This part is privately owned by a worker thread.
/// The GC controller also has an embedded `GCWorker` because it may also execute work packets.
pub struct GCWorker<VM: VMBinding> {
    /// The VM-specific thread-local state of the GC thread.
    pub tls: VMWorkerThread,
    /// The ordinal of the worker, numbered from 0 to the number of workers minus one.
    /// 0 if it is the embedded worker of the GC controller thread.
    pub ordinal: usize,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The copy context, used to implement copying GC.
    copy: GCWorkerCopyContext<VM>,
    /// The sending end of the channel to send message to the controller thread.
    pub sender: Sender<CoordinatorMessage<VM>>,
    /// The reference to the MMTk instance.
    pub mmtk: &'static MMTK<VM>,
    /// True if this struct is the embedded GCWorker of the controller thread.
    /// False if this struct belongs to a standalone GCWorker thread.
    is_coordinator: bool,
    /// Reference to the shared part of the GC worker.  It is used for synchronization.
    pub shared: Arc<GCWorkerShared<VM>>,
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
    pub fn new(
        mmtk: &'static MMTK<VM>,
        ordinal: usize,
        scheduler: Arc<GCWorkScheduler<VM>>,
        is_coordinator: bool,
        sender: Sender<CoordinatorMessage<VM>>,
        shared: Arc<GCWorkerShared<VM>>,
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
        }
    }

    const LOCALLY_CACHED_WORK_PACKETS: usize = 16;

    /// Add a work packet to the work queue and mark it with a higher priority.
    /// If the bucket is activated, the packet will be pushed to the local queue, otherwise it will be
    /// pushed to the global bucket with a higher priority.
    #[inline]
    pub fn add_work_prioritized(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || self.shared.local_work_buffer.len() >= Self::LOCALLY_CACHED_WORK_PACKETS
        {
            self.scheduler.work_buckets[bucket].add_prioritized(Box::new(work));
            return;
        }
        self.shared.local_work_buffer.push(Box::new(work));
    }

    /// Add a work packet to the work queue.
    /// If the bucket is activated, the packet will be pushed to the local queue, otherwise it will be
    /// pushed to the global bucket.
    #[inline]
    pub fn add_work(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || self.shared.local_work_buffer.len() >= Self::LOCALLY_CACHED_WORK_PACKETS
        {
            self.scheduler.work_buckets[bucket].add(work);
            return;
        }
        self.shared.local_work_buffer.push(Box::new(work));
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
            .local_work
            .pop()
            .or_else(|| {
                self.shared
                    .local_work_buffer
                    .pop()
                    .or_else(|| Some(self.scheduler().poll(self)))
            })
            .unwrap()
    }

    pub fn do_boxed_work(&'static mut self, mut work: Box<dyn GCWork<VM>>) {
        work.do_work(self, self.mmtk);
    }

    /// Entry of the worker thread.
    /// Each worker will keep polling and executing work packets in a loop.
    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        self.tls = tls;
        self.copy = crate::plan::create_gc_worker_context(tls, mmtk);
        loop {
            let mut work = self.poll();
            work.do_work_with_stat(self, mmtk);
        }
    }
}

/// A worker group to manage all the GC workers (except the coordinator worker).
pub struct WorkerGroup<VM: VMBinding> {
    /// Shared worker data
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
    /// Handles for stealing packets from workers
    pub stealers: Vec<Stealer<Box<dyn GCWork<VM>>>>,
    parked_workers: AtomicUsize,
}

impl<VM: VMBinding> WorkerGroup<VM> {
    /// Create a WorkerGroup
    pub fn new(num_workers: usize) -> Arc<Self> {
        let workers_shared = (0..num_workers)
            .map(|_| Arc::new(GCWorkerShared::<VM>::new()))
            .collect::<Vec<_>>();

        let stealers = workers_shared
            .iter()
            .map(|worker| worker.local_work_buffer.stealer())
            .collect();

        Arc::new(Self {
            workers_shared,
            stealers,
            parked_workers: Default::default(),
        })
    }

    /// Spawn all the worker threads
    pub fn spawn(
        &self,
        mmtk: &'static MMTK<VM>,
        sender: Sender<CoordinatorMessage<VM>>,
        tls: VMThread,
    ) {
        // Spawn each worker thread.
        for (ordinal, shared) in self.workers_shared.iter().enumerate() {
            let worker = Box::new(GCWorker::new(
                mmtk,
                ordinal,
                mmtk.scheduler.clone(),
                false,
                sender.clone(),
                shared.clone(),
            ));
            VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Worker(worker));
        }
    }

    /// Get the number of workers in the group
    #[inline(always)]
    pub fn worker_count(&self) -> usize {
        self.workers_shared.len()
    }

    /// Increase the packed-workers counter.
    /// Called before a worker is parked.
    ///
    /// Return true if all the workers are parked.
    #[inline(always)]
    pub fn inc_parked_workers(&self) -> bool {
        let old = self.parked_workers.fetch_add(1, Ordering::Relaxed);
        debug_assert!(old < self.worker_count());
        old + 1 == self.worker_count()
    }

    /// Decrease the packed-workers counter.
    /// Called after a worker is resumed from the parked state.
    #[inline(always)]
    pub fn dec_parked_workers(&self) {
        let old = self.parked_workers.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old <= self.worker_count());
    }

    /// Get the number of parked workers in the group
    #[inline(always)]
    pub fn parked_workers(&self) -> usize {
        self.parked_workers.load(Ordering::Relaxed)
    }

    /// Check if all the workers are packed
    #[inline(always)]
    pub fn all_parked(&self) -> bool {
        self.parked_workers() == self.worker_count()
    }
}
