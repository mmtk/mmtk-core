use std::sync::{Mutex, RwLock, Condvar, Arc};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::LinkedList;
use std::ptr;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use crate::util::OpaquePointer;
use super::work::{GCWork, Work};
use super::worker::{WorkerGroup, Worker};
use crate::vm::Collection;
use std::collections::BinaryHeap;
use std::cmp;
use crate::plan::Plan;
use super::work_bucket::*;
use super::*;



pub struct Scheduler<C: Context> {
    /// Works that are scheduable at any time
    pub unconstrained_works: WorkBucket<C>,
    /// Works that are scheduable within Stop-the-world
    pub prepare_stage: WorkBucket<C>,
    pub closure_stage: WorkBucket<C>,
    pub release_stage: WorkBucket<C>,
    pub final_stage: WorkBucket<C>,
    /// workers
    worker_group: Option<Arc<WorkerGroup<C>>>,
    /// Condition Variable
    pub monitor: Arc<(Mutex<()>, Condvar)>,
}

impl <C: Context> Scheduler<C> {
    pub fn new() -> Arc<Self> {
        let monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        Arc::new(Self {
            unconstrained_works: WorkBucket::new(true, monitor.clone()), // `default_bucket` is always activated
            prepare_stage: WorkBucket::new(false, monitor.clone()),
            closure_stage: WorkBucket::new(false, monitor.clone()),
            release_stage: WorkBucket::new(false, monitor.clone()),
            final_stage: WorkBucket::new(false, monitor.clone()),
            worker_group: None,
            monitor,
        })
    }

    pub fn initialize(self: &'static Arc<Self>, num_workers: usize, tls: OpaquePointer) {
        let mut self_mut = self.clone();
        let self_mut = unsafe { Arc::get_mut_unchecked(&mut self_mut) };

        self_mut.worker_group = Some(WorkerGroup::new(1, Arc::downgrade(&self)));
        self.worker_group.as_ref().unwrap().spawn_workers(tls);

        self_mut.closure_stage.set_open_condition(move || {
            self.prepare_stage.is_drained() && self.worker_group().all_parked()
        });
        self_mut.release_stage.set_open_condition(move || {
            self.closure_stage.is_drained() && self.worker_group().all_parked()
        });
        self_mut.final_stage.set_open_condition(move || {
            self.release_stage.is_drained() && self.worker_group().all_parked()
        });
    }

    pub fn worker_group(&self) -> Arc<WorkerGroup<C>> {
        self.worker_group.as_ref().unwrap().clone()
    }

    fn all_buckets_drained(&self) -> bool {
        self.unconstrained_works.is_drained()
        && self.prepare_stage.is_drained()
        && self.closure_stage.is_drained()
        && self.release_stage.is_drained()
        && self.final_stage.is_drained()
    }

    pub fn wait_for_completion(&self) {
        let mut guard = self.monitor.0.lock().unwrap();
        loop {
            if self.prepare_stage.update() {
                println!("prepare_stage open");
                self.monitor.1.notify_all();
            }
            if self.closure_stage.update() {
                println!("closure_stage open");
                self.monitor.1.notify_all();
            }
            if self.release_stage.update() {
                println!("release_stage open");
                self.monitor.1.notify_all();
            }
            if self.final_stage.update() {
                println!("final_stage open");
                self.monitor.1.notify_all();
            }
            if self.worker_group().all_parked() && self.all_buckets_drained() {
                break;
            }
            guard = self.monitor.1.wait(guard).unwrap();
        }
        self.prepare_stage.deactivate();
        self.closure_stage.deactivate();
        self.release_stage.deactivate();
        self.final_stage.deactivate();
    }

    fn pop_scheduable_work(&self, worker: &Worker<C>) -> Option<Box<dyn Work<C>>> {
        if let Some(work) = worker.local_works.poll() {
            return Some(work);
        }
        if let Some(work) = self.unconstrained_works.poll() {
            return Some(work);
        }
        if let Some(work) = self.prepare_stage.poll() {
            return Some(work);
        }
        if let Some(work) = self.closure_stage.poll() {
            return Some(work);
        }
        if let Some(work) = self.release_stage.poll() {
            return Some(work);
        }
        if let Some(work) = self.final_stage.poll() {
            return Some(work);
        }
        None
    }

    /// Get a scheduable work. Called by workers
    pub fn poll(&self, worker: &Worker<C>) -> Box<dyn Work<C>> {
        debug_assert!(!worker.is_parked());
        let mut guard = self.monitor.0.lock().unwrap();
        loop {
            debug_assert!(!worker.is_parked());
            if let Some(work) = self.pop_scheduable_work(worker) {
                self.monitor.1.notify_all();
                return work;
            }
            // Park this worker
            println!("Park");
            worker.parked.store(true, Ordering::SeqCst);
            self.monitor.1.notify_all();
            println!("Park Notified");
            // Wait
            guard = self.monitor.1.wait(guard).unwrap();
            // Unpark this worker
            println!("UnPark");
            worker.parked.store(false, Ordering::SeqCst);
            self.monitor.1.notify_all();
        }
    }
}

pub type MMTkScheduler<VM> = Scheduler<MMTK<VM>>;

impl <VM: VMBinding> MMTkScheduler<VM> {
    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().control_collector_context.as_ref().unwrap().clear_request();
        self.prepare_stage.activate();
    }
}