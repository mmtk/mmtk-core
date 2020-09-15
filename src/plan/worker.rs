use crate::vm::VMBinding;
use super::scheduler::*;
use crate::util::OpaquePointer;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::vm::Collection;
use crate::mmtk::MMTK;
use crate::{SelectedPlan, Plan, CopyContext};



pub struct Worker<VM: VMBinding> {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    group: Weak<WorkerGroup<VM>>,
    scheduler: Weak<Scheduler<VM>>,
    pub context: Option<<SelectedPlan::<VM> as Plan>::CopyContext>,
    pub local_works: WorkBucket<VM>,
}

impl <VM: VMBinding> Worker<VM> {
    fn new(ordinal: usize, group: Weak<WorkerGroup<VM>>, scheduler: Weak<Scheduler<VM>>) -> Self {
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            group,
            context: None,
            local_works: WorkBucket::new(true, scheduler.upgrade().unwrap().monitor.clone()),
            scheduler,
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn group(&self) -> Arc<WorkerGroup<VM>> {
        self.group.upgrade().unwrap()
    }

    pub fn scheduler(&self) -> Arc<Scheduler<VM>> {
        self.scheduler.upgrade().unwrap()
    }

    pub fn context(&self) -> &mut <SelectedPlan::<VM> as Plan>::CopyContext {
        unsafe { &mut *(self.context.as_ref().unwrap() as *const _ as *mut _) }
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn run(&'static mut self, mmtk: &'static MMTK<VM>) {
        self.context = Some(<SelectedPlan::<VM> as Plan>::CopyContext::new(mmtk));
        self.parked.store(false, Ordering::SeqCst);
        let scheduler = self.scheduler.upgrade().unwrap();
        loop {
            let mut work = scheduler.poll(self);
            debug_assert!(!self.is_parked());
            let this = unsafe { &mut *(self as *mut _) };
            work.do_work(this, mmtk);
        }
    }
}


pub struct WorkerGroup<VM: VMBinding> {
    pub workers: Vec<Worker<VM>>,
}

impl <VM: VMBinding> WorkerGroup<VM> {
    pub fn new(workers: usize, scheduler: Weak<Scheduler<VM>>) -> Arc<Self> {
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
            VM::VMCollection::spawn_worker_thread(tls, Some(worker));
        }
    }
}
