use super::*;
use super::work_bucket::*;
use crate::util::OpaquePointer;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::mmtk::MMTK;




pub struct Worker<C: Context> {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    group: Weak<WorkerGroup<C>>,
    scheduler: Weak<Scheduler<C>>,
    local: Option<C::WorkerLocal>,
    pub local_works: WorkBucket<C>,
}

pub type GCWorker<VM> = Worker<MMTK<VM>>;

impl <C: Context> Worker<C> {
    fn new(ordinal: usize, group: Weak<WorkerGroup<C>>, scheduler: Weak<Scheduler<C>>) -> Self {
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            group,
            local: None,
            local_works: WorkBucket::new(true, scheduler.upgrade().unwrap().monitor.clone()),
            scheduler,
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn group(&self) -> Arc<WorkerGroup<C>> {
        self.group.upgrade().unwrap()
    }

    pub fn scheduler(&self) -> Arc<Scheduler<C>> {
        self.scheduler.upgrade().unwrap()
    }

    pub fn local(&self) -> &mut C::WorkerLocal {
        unsafe { &mut *(self.local.as_ref().unwrap() as *const _ as *mut _) }
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn run(&'static mut self, context: &'static C) {
        self.local = Some(C::WorkerLocal::new(context));
        self.parked.store(false, Ordering::SeqCst);
        let scheduler = self.scheduler.upgrade().unwrap();
        loop {
            let mut work = scheduler.poll(self);
            debug_assert!(!self.is_parked());
            let this = unsafe { &mut *(self as *mut _) };
            work.do_work(this, context);
        }
    }
}


pub struct WorkerGroup<C: Context> {
    pub workers: Vec<Worker<C>>,
}

impl <C: Context> WorkerGroup<C> {
    pub fn new(workers: usize, scheduler: Weak<Scheduler<C>>) -> Arc<Self> {
        let mut group = Arc::new(Self {
            workers: vec![]
        });
        let group_weak = Arc::downgrade(&group);
        unsafe { Arc::get_mut_unchecked(&mut group) }.workers = (0..workers).map(|i| Worker::new(i, group_weak.clone(), scheduler.clone())).collect();
        group
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

    pub fn spawn_workers(&self, tls: OpaquePointer) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            C::spawn_worker(worker, tls);
        }
    }
}
