use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::copy::GCWorkerCopyContext;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, VMBinding};
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

const LOCALLY_CACHED_WORKS: usize = 1;

/// The part shared between a GCWorker and the scheduler.
/// This structure is used for communication, e.g. adding new work packets.
pub struct GCWorkerShared<VM: VMBinding> {
    pub parked: AtomicBool,
    stat: AtomicRefCell<WorkerLocalStat<VM>>,
    pub local_work_bucket: WorkBucket<VM>,
}

/// A GC worker.  This part is privately owned by a worker thread.
pub struct GCWorker<VM: VMBinding> {
    pub tls: VMWorkerThread,
    pub ordinal: usize,
    scheduler: Arc<GCWorkScheduler<VM>>,
    copy: GCWorkerCopyContext<VM>,
    pub sender: Sender<CoordinatorMessage<VM>>,
    mmtk: Option<&'static MMTK<VM>>,
    is_coordinator: bool,
    local_work_buffer: Vec<(WorkBucketStage, Box<dyn GCWork<VM>>)>,
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
        ordinal: usize,
        scheduler: Arc<GCWorkScheduler<VM>>,
        is_coordinator: bool,
        sender: Sender<CoordinatorMessage<VM>>,
    ) -> Self {
        let worker_monitor = scheduler.worker_monitor.clone();
        Self {
            tls: VMWorkerThread(VMThread::UNINITIALIZED),
            ordinal,
            // We will set this later
            copy: GCWorkerCopyContext::new_non_copy(),
            sender,
            scheduler,
            mmtk: None,
            is_coordinator,
            local_work_buffer: Vec::with_capacity(LOCALLY_CACHED_WORKS),
            shared: Arc::new(GCWorkerShared {
                parked: AtomicBool::new(true),
                stat: Default::default(),
                local_work_bucket: WorkBucket::new(true, worker_monitor),
            }),
        }
    }

    #[inline]
    pub fn add_work(&mut self, bucket: WorkBucketStage, work: impl GCWork<VM>) {
        if !self.scheduler().work_buckets[bucket].is_activated() {
            self.scheduler.work_buckets[bucket].add_with_priority(1000, box work);
            return;
        }
        self.local_work_buffer.push((bucket, box work));
        if self.local_work_buffer.len() > LOCALLY_CACHED_WORKS {
            self.flush();
        }
    }

    #[cold]
    fn flush(&mut self) {
        let mut buffer = Vec::with_capacity(LOCALLY_CACHED_WORKS);
        std::mem::swap(&mut buffer, &mut self.local_work_buffer);
        for (bucket, work) in buffer {
            self.scheduler.work_buckets[bucket].add_with_priority(1000, work);
        }
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
        work.do_work(self, self.mmtk.unwrap());
    }

    pub fn run(&mut self, tls: VMWorkerThread, mmtk: &'static MMTK<VM>) {
        self.tls = tls;
        self.copy = crate::plan::create_gc_worker_context(tls, mmtk);
        self.mmtk = Some(mmtk);
        self.shared.parked.store(false, Ordering::SeqCst);
        loop {
            while let Some((bucket, mut work)) = self.local_work_buffer.pop() {
                debug_assert!(self.scheduler.work_buckets[bucket].is_activated());
                work.do_work_with_stat(self, mmtk);
            }
            let mut work = self.scheduler().poll(self);
            debug_assert!(!self.shared.is_parked());
            work.do_work_with_stat(self, mmtk);
        }
    }
}

pub struct WorkerGroup<VM: VMBinding> {
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
}

impl<VM: VMBinding> WorkerGroup<VM> {
    pub fn new(
        workers: usize,
        scheduler: Arc<GCWorkScheduler<VM>>,
        sender: Sender<CoordinatorMessage<VM>>,
    ) -> (Self, Box<dyn FnOnce(VMThread)>) {
        let mut workers_shared = Vec::new();
        let mut workers_to_spawn = Vec::new();

        for ordinal in 0..workers {
            let worker = Box::new(GCWorker::new(
                ordinal,
                scheduler.clone(),
                false,
                sender.clone(),
            ));
            let worker_shared = worker.shared.clone();
            workers_shared.push(worker_shared);
            workers_to_spawn.push(worker);
        }

        // NOTE: We cannot call spawn_worker_thread here,
        // because the worker will access `Scheduler::worker_group` immediately after started,
        // but that field will not be assigned to before this function returns.
        // Therefore we defer the spawning operation later.
        let deferred_spawn = Box::new(move |tls| {
            for worker in workers_to_spawn.drain(..) {
                VM::VMCollection::spawn_worker_thread(tls, Some(worker));
            }
        });

        (Self { workers_shared }, deferred_spawn)
    }

    pub fn worker_count(&self) -> usize {
        self.workers_shared.len()
    }

    pub fn parked_workers(&self) -> usize {
        self.workers_shared.iter().filter(|w| w.is_parked()).count()
    }

    pub fn all_parked(&self) -> bool {
        self.parked_workers() == self.worker_count()
    }
}
