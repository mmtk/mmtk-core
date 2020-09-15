use std::sync::{Mutex, RwLock, Condvar, Arc};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::LinkedList;
use std::ptr;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use crate::util::OpaquePointer;
use super::work::{Work, GenericWork};
use super::worker::{WorkerGroup, Worker};
use crate::vm::Collection;
use std::collections::BinaryHeap;
use std::cmp;


// #[derive(Eq, PartialEq)]
struct PrioritizedWork<VM: VMBinding> {
    priority: usize,
    work: Box<dyn GenericWork<VM>>,
}

impl <VM: VMBinding> PartialEq for PrioritizedWork<VM> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && &self.work == &other.work
    }
}

impl <VM: VMBinding> Eq for PrioritizedWork<VM> {}

impl <VM: VMBinding> Ord for PrioritizedWork<VM> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // other.0.cmp(&self.0)
        self.priority.cmp(&other.priority)
    }
}

impl <VM: VMBinding> PartialOrd for PrioritizedWork<VM> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct WorkBucket<VM: VMBinding> {
    active: AtomicBool,
    /// A priority queue
    queue: RwLock<BinaryHeap<PrioritizedWork<VM>>>,
    monitor: Arc<(Mutex<()>, Condvar)>,
    pub active_priority: AtomicUsize,
    can_open: Option<Box<dyn Fn() -> bool>>,
    can_close: Option<Box<dyn Fn()>>,
}

unsafe impl <VM: VMBinding> Send for WorkBucket<VM> {}
unsafe impl <VM: VMBinding> Sync for WorkBucket<VM> {}

impl <VM: VMBinding> WorkBucket<VM> {
    pub fn new(active: bool, monitor: Arc<(Mutex<()>, Condvar)>) -> Self {
        Self {
            active: AtomicBool::new(active),
            queue: Default::default(),
            monitor,
            active_priority: AtomicUsize::new(usize::max_value()),
            can_open: None,
            can_close: None,
        }
    }
    pub fn is_activated(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    pub fn active_priority(&self) -> usize {
        self.active_priority.load(Ordering::SeqCst)
    }
    /// Enable the bucket
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }
    /// Test if the bucket is drained
    pub fn is_empty(&self) -> bool {
        self.queue.read().unwrap().len() == 0
    }
    pub fn is_drained(&self) -> bool {
        self.is_activated() && self.is_empty()
    }
    /// Disable the bucket
    pub fn deactivate(&self) {
        debug_assert!(self.queue.read().unwrap().is_empty(), "Bucket not drained before close");
        self.active.store(false, Ordering::SeqCst);
        self.active_priority.store(usize::max_value(), Ordering::SeqCst);
    }
    /// Add a work packet to this bucket
    pub fn add_with_priority(&self, priority: usize, work: Box<dyn GenericWork<VM>>) {
        let _guard = self.monitor.0.lock().unwrap();
        self.monitor.1.notify_all();
        self.queue.write().unwrap().push(PrioritizedWork { priority, work });
    }
    pub fn add(&self, work: Box<dyn GenericWork<VM>>) {
        self.add_with_priority(usize::max_value(), work);
    }
    // pub fn add(&self, priority: usize, work: Box<dyn GenericWork<VM>>) {
    //     let _guard = self.monitor.0.lock().unwrap();
    //     self.monitor.1.notify_all();
    //     self.queue.write().unwrap().push(PrioritizedWork { priority, work });
    // }
    // pub fn add_with_highest_priority(&self, work: Box<dyn GenericWork<VM>>) -> usize {
    //     let priority = usize::max_value();
    //     self.add(priority, work);
    //     priority
    // }
    /// Get a work packet (with the greatest priority) from this bucket
    fn poll(&self) -> Option<Box<dyn GenericWork<VM>>> {
        if !self.active.load(Ordering::SeqCst) { return None }
        self.queue.write().unwrap().pop().map(|v| v.work)
    }
    pub fn set_open_condition(&mut self, pred: impl Fn() -> bool + 'static) {
        self.can_open = Some(box pred);
    }
    pub fn update(&self) -> bool {
        if let Some(can_open) = self.can_open.as_ref() {
            if !self.is_activated() && can_open() {
                self.activate();
                return true;
            }
        }
        false
    }
}

pub enum ScheduleStage {
    Default,
    Prepare,
    Closure,
    Release,
    Final,
}

pub struct Scheduler<VM: VMBinding> {
    /// Works that are scheduable at any time
    pub unconstrained_works: WorkBucket<VM>,
    /// Works that are scheduable within Stop-the-world
    pub prepare_stage: WorkBucket<VM>,
    pub closure_stage: WorkBucket<VM>,
    pub release_stage: WorkBucket<VM>,
    pub final_stage: WorkBucket<VM>,
    /// workers
    worker_group: Option<Arc<WorkerGroup<VM>>>,
    /// Condition Variable
    pub monitor: Arc<(Mutex<()>, Condvar)>,
}

impl <VM: VMBinding> Scheduler<VM> {
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

    pub fn initialize(&'static mut self, mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
        let size = 1;//mmtk.options.threads;

        self.worker_group = Some(WorkerGroup::new(size, Arc::downgrade(&mmtk.scheduler)));
        self.worker_group.as_ref().unwrap().spawn_workers(tls);

        self.closure_stage.set_open_condition(move || {
            mmtk.scheduler.prepare_stage.is_drained() && mmtk.scheduler.worker_group().all_parked()
        });
        self.release_stage.set_open_condition(move || {
            mmtk.scheduler.closure_stage.is_drained() && mmtk.scheduler.worker_group().all_parked()
        });
        self.final_stage.set_open_condition(move || {
            mmtk.scheduler.release_stage.is_drained() && mmtk.scheduler.worker_group().all_parked()
        });
    }

    pub fn worker_group(&self) -> Arc<WorkerGroup<VM>> {
        self.worker_group.as_ref().unwrap().clone()
    }

    pub fn add<W: Work<VM=VM>>(&self, stage: ScheduleStage, work: W) {
        match stage {
            ScheduleStage::Default => self.unconstrained_works.add(box work),
            ScheduleStage::Prepare => self.prepare_stage.add(box work),
            ScheduleStage::Closure => self.closure_stage.add(box work),
            ScheduleStage::Release => self.release_stage.add(box work),
            ScheduleStage::Final => self.final_stage.add(box work),
        }
    }

    // pub fn add<W: Work<VM=VM>>(&self, priority: usize, work: W) {
    //     if W::REQUIRES_STOP_THE_WORLD {
    //         self.stw_bucket.add(priority, box work);
    //     } else {
    //         self.default_bucket.add(priority, box work);
    //     }
    // }

    // pub fn add_with_highest_priority<W: Work<VM=VM>>(&self, work: W) -> usize {
    //     if W::REQUIRES_STOP_THE_WORLD {
    //         self.stw_bucket.add_with_highest_priority(box work)
    //     } else {
    //         self.default_bucket.add_with_highest_priority(box work)
    //     }
    // }

    pub fn mutators_stopped(&self) {
        println!("mutators_stopped");
        self.prepare_stage.activate()
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

    fn pop_scheduable_work(&self, worker: &Worker<VM>) -> Option<Box<dyn GenericWork<VM>>> {
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
    pub fn poll(&self, worker: &Worker<VM>) -> Box<dyn GenericWork<VM>> {
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
