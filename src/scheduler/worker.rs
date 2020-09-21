use super::*;
use super::work_bucket::*;
use crate::util::OpaquePointer;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use crate::mmtk::MMTK;




pub struct Worker<C: Context> {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    group: Option<Arc<WorkerGroup<C>>>,
    scheduler: Arc<Scheduler<C>>,
    local: Option<C::WorkerLocal>,
    pub local_works: WorkBucket<C>,
    pub packets: usize,
    pub sender: Sender<CoordinatorMessage<C>>,
}

unsafe impl <C: Context> Sync for Worker<C> {}
unsafe impl <C: Context> Send for Worker<C> {}

pub type GCWorker<VM> = Worker<MMTK<VM>>;

impl <C: Context> Worker<C> {
    pub fn new(ordinal: usize, group: Option<Weak<WorkerGroup<C>>>, scheduler: Weak<Scheduler<C>>) -> Self {
        let scheduler = scheduler.upgrade().unwrap();
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            group: group.map(|g| g.upgrade().unwrap()),
            local: None,
            local_works: WorkBucket::new(true, scheduler.worker_monitor.clone()),
            sender: scheduler.channel.0.clone(),
            scheduler: scheduler,
            packets: 0,
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn group(&self) -> Option<&WorkerGroup<C>> {
        self.group.as_ref().map(|g| &g as &WorkerGroup<C>)
    }

    pub fn is_coordinator(&self) -> bool {
        self.group.is_none()
    }

    pub fn scheduler(&self) -> &Scheduler<C> {
        &self.scheduler
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
        loop {
            let mut work = self.scheduler().poll(self);
            if cfg!(debug_assertions) {
                self.packets += 1;
            }
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
        unsafe { Arc::get_mut_unchecked(&mut group) }.workers = (0..workers).map(|i| Worker::new(i, Some(group_weak.clone()), scheduler.clone())).collect();
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

    pub fn spawn_workers(&'static self, tls: OpaquePointer, context: &'static C) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            C::spawn_worker(worker, tls, context);
        }
    }
}
