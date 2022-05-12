use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::copy::GCWorkerCopyContext;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, GCThreadContext, VMBinding};
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use crossbeam::deque::{Stealer, Worker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// The part shared between a GCWorker and the scheduler.
/// This structure is used for communication, e.g. adding new work packets.
pub struct GCWorkerShared<VM: VMBinding> {
    /// True if the GC worker is parked.
    pub parked: AtomicBool,
    /// Worker-local statistics data.
    stat: AtomicRefCell<WorkerLocalStat<VM>>,
    pub local_work: Worker<Box<dyn GCWork<VM>>>,
    /// Cache of work packets created by the current worker.
    /// May be flushed to the global pool or executed locally.
    pub local_work_buffer: Worker<Box<dyn GCWork<VM>>>,
}

impl<VM: VMBinding> GCWorkerShared<VM> {
    pub fn new() -> Self {
        Self {
            parked: AtomicBool::new(true),
            stat: Default::default(),
            local_work: Worker::new_fifo(),
            local_work_buffer: Worker::new_fifo(),
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
    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

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

    #[inline]
    pub fn add_work_prioritized(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || !self.shared.local_work_buffer.is_empty()
        {
            self.scheduler.work_buckets[bucket].add_prioritized(Box::new(work));
            return;
        }
        self.shared.local_work_buffer.push(Box::new(work));
    }

    #[inline]
    pub fn add_work(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated()
            || !self.shared.local_work_buffer.is_empty()
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

    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        self.tls = tls;
        self.copy = crate::plan::create_gc_worker_context(tls, mmtk);
        self.shared.parked.store(false, Ordering::SeqCst);
        loop {
            let mut work = self.poll();
            debug_assert!(!self.shared.is_parked());
            work.do_work_with_stat(self, mmtk);
        }
    }
}

pub struct WorkerGroup<VM: VMBinding> {
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
    pub stealers: Vec<(usize, Stealer<Box<dyn GCWork<VM>>>)>,
    parked_workers: AtomicUsize,
}

impl<VM: VMBinding> WorkerGroup<VM> {
    pub fn new(num_workers: usize) -> Arc<Self> {
        let workers_shared = (0..num_workers)
            .map(|_| Arc::new(GCWorkerShared::<VM>::new()))
            .collect::<Vec<_>>();

        let stealers = workers_shared
            .iter()
            .zip(0..num_workers)
            .map(|(w, ordinal)| (ordinal, w.local_work_buffer.stealer()))
            .collect();

        Arc::new(Self {
            workers_shared,
            stealers,
            parked_workers: Default::default(),
        })
    }

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

    #[inline(always)]
    pub fn worker_count(&self) -> usize {
        self.workers_shared.len()
    }

    #[inline(always)]
    pub fn inc_parked_workers(&self) -> bool {
        let old = self.parked_workers.fetch_add(1, Ordering::SeqCst);
        old + 1 == self.worker_count()
    }

    #[inline(always)]
    pub fn dec_parked_workers(&self) {
        self.parked_workers.fetch_sub(1, Ordering::SeqCst);
    }

    #[inline(always)]
    pub fn parked_workers(&self) -> usize {
        self.parked_workers.load(Ordering::SeqCst)
    }

    #[inline(always)]
    pub fn all_parked(&self) -> bool {
        self.parked_workers() == self.worker_count()
    }
}
