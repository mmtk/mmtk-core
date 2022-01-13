use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::copy::GCWorkerCopyContext;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, VMBinding};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

const LOCALLY_CACHED_WORKS: usize = 1;

pub struct GCWorker<VM: VMBinding> {
    pub tls: VMWorkerThread,
    pub ordinal: usize,
    pub parked: AtomicBool,
    scheduler: Arc<GCWorkScheduler<VM>>,
    copy: GCWorkerCopyContext<VM>,
    pub local_work_bucket: WorkBucket<VM>,
    pub sender: Sender<CoordinatorMessage<VM>>,
    pub stat: WorkerLocalStat<VM>,
    mmtk: Option<&'static MMTK<VM>>,
    is_coordinator: bool,
    local_work_buffer: Vec<(WorkBucketStage, Box<dyn GCWork<VM>>)>,
}

unsafe impl<VM: VMBinding> Sync for GCWorker<VM> {}
unsafe impl<VM: VMBinding> Send for GCWorker<VM> {}

impl<VM: VMBinding> GCWorker<VM> {
    pub fn new(
        ordinal: usize,
        scheduler: Weak<GCWorkScheduler<VM>>,
        is_coordinator: bool,
        sender: Sender<CoordinatorMessage<VM>>,
    ) -> Self {
        let scheduler = scheduler.upgrade().unwrap();
        Self {
            tls: VMWorkerThread(VMThread::UNINITIALIZED),
            ordinal,
            parked: AtomicBool::new(true),
            // We will set this later
            copy: GCWorkerCopyContext::new_non_copy(),
            local_work_bucket: WorkBucket::new(true, scheduler.worker_monitor.clone()),
            sender,
            scheduler,
            stat: Default::default(),
            mmtk: None,
            is_coordinator,
            local_work_buffer: Vec::with_capacity(LOCALLY_CACHED_WORKS),
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

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
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
        self.parked.store(false, Ordering::SeqCst);
        loop {
            while let Some((bucket, mut work)) = self.local_work_buffer.pop() {
                debug_assert!(self.scheduler.work_buckets[bucket].is_activated());
                work.do_work_with_stat(self, mmtk);
            }
            let mut work = self.scheduler().poll(self);
            debug_assert!(!self.is_parked());
            work.do_work_with_stat(self, mmtk);
        }
    }
}

pub struct WorkerGroup<VM: VMBinding> {
    pub workers: Vec<GCWorker<VM>>,
}

impl<VM: VMBinding> WorkerGroup<VM> {
    pub fn new(
        workers: usize,
        scheduler: Weak<GCWorkScheduler<VM>>,
        sender: Sender<CoordinatorMessage<VM>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workers: (0..workers)
                .map(|i| GCWorker::new(i, scheduler.clone(), false, sender.clone()))
                .collect(),
        })
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn parked_workers(&self) -> usize {
        self.workers.iter().filter(|w| w.is_parked()).count()
    }

    pub fn all_parked(&self) -> bool {
        self.parked_workers() == self.worker_count()
    }

    pub fn spawn_workers(&'static self, tls: VMThread, _mmtk: &'static MMTK<VM>) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            VM::VMCollection::spawn_worker_thread(tls, Some(worker));
        }
    }
}
