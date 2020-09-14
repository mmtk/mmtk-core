use crate::vm::VMBinding;
use super::scheduler::*;
use crate::util::OpaquePointer;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::vm::Collection;



pub struct Worker {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    group: Weak<WorkerGroup>,
    scheduler: Weak<Scheduler>,
}

impl Worker {
    fn new(ordinal: usize, group: Weak<WorkerGroup>, scheduler: Weak<Scheduler>) -> Self {
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            group,
            scheduler,
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn group(&self) -> Arc<WorkerGroup> {
        self.group.upgrade().unwrap()
    }

    pub fn scheduler(&self) -> Arc<Scheduler> {
        self.scheduler.upgrade().unwrap()
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn run(&mut self) {
        self.parked.store(false, Ordering::SeqCst);
        let scheduler = self.scheduler.upgrade().unwrap();
        loop {
            let mut work = scheduler.poll(self);
            debug_assert!(!self.is_parked());
            let static_scheduler: &'static Scheduler = unsafe {
                &*(scheduler.as_ref() as *const Scheduler)
            };
            work.do_work(self, static_scheduler);
        }
    }
}


pub struct WorkerGroup {
    pub workers: Vec<Worker>,
}

impl WorkerGroup {
    pub fn new(workers: usize, scheduler: Weak<Scheduler>) -> Arc<Self> {
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

    pub fn spawn_workers<VM: VMBinding>(&self, tls: OpaquePointer) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            VM::VMCollection::spawn_worker_thread(tls, Some(worker));
        }
    }
}
