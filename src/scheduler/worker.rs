use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::OpaquePointer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

pub struct Worker<C: Context> {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    scheduler: Arc<Scheduler<C>>,
    local: Option<C::WorkerLocal>,
    pub local_works: WorkBucket<C>,
    pub sender: Sender<CoordinatorMessage<C>>,
    pub stat: WorkerLocalStat,
    context: Option<&'static C>,
    is_coordinator: bool,
}

unsafe impl<C: Context> Sync for Worker<C> {}
unsafe impl<C: Context> Send for Worker<C> {}

pub type GCWorker<VM> = Worker<MMTK<VM>>;

impl<C: Context> Worker<C> {
    pub fn new(ordinal: usize, scheduler: Weak<Scheduler<C>>, is_coordinator: bool) -> Self {
        let scheduler = scheduler.upgrade().unwrap();
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            local: None,
            local_works: WorkBucket::new(true, scheduler.worker_monitor.clone()),
            sender: scheduler.channel.0.clone(),
            scheduler,
            stat: Default::default(),
            context: None,
            is_coordinator,
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn is_coordinator(&self) -> bool {
        self.is_coordinator
    }

    pub fn scheduler(&self) -> &Scheduler<C> {
        &self.scheduler
    }

    #[inline]
    pub fn local(&mut self) -> &mut C::WorkerLocal {
        self.local.as_mut().unwrap()
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn do_work(&'static mut self, mut work: impl Work<C>) {
        work.do_work(self, self.context.unwrap());
    }

    pub fn run(&mut self, context: &'static C) {
        self.context = Some(context);
        self.local = Some(C::WorkerLocal::new(context));
        let tls = self.tls;
        self.local().init(tls);
        self.parked.store(false, Ordering::SeqCst);
        loop {
            let mut work = self.scheduler().poll(self);
            debug_assert!(!self.is_parked());
            work.do_work_with_stat(self, context);
        }
    }
}

pub struct WorkerGroup<C: Context> {
    pub workers: Vec<Worker<C>>,
}

impl<C: Context> WorkerGroup<C> {
    pub fn new(workers: usize, scheduler: Weak<Scheduler<C>>) -> Arc<Self> {
        Arc::new(Self {
            workers: (0..workers)
                .map(|i| Worker::new(i, scheduler.clone(), false))
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

    pub fn spawn_workers(&'static self, tls: OpaquePointer, context: &'static C) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            C::spawn_worker(worker, tls, context);
        }
    }
}
