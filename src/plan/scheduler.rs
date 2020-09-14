use std::sync::{Mutex, RwLock, Condvar, Arc};
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl <VM: VMBinding> WorkBucket<VM> {
    pub fn new(active: bool, monitor: Arc<(Mutex<()>, Condvar)>) -> Self {
        Self {
            active: AtomicBool::new(active),
            queue: Default::default(),
            monitor,
        }
    }
    /// Enable the bucket
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }
    /// Test if the bucket is drained
    pub fn is_empty(&self) -> bool {
        self.queue.read().unwrap().len() == 0
    }
    /// Disable the bucket
    pub fn deactivate(&self) {
        debug_assert!(self.queue.read().unwrap().is_empty(), "Bucket not drained before close");
        self.active.store(false, Ordering::SeqCst);
    }
    /// Add a work packet to this bucket
    pub fn add(&self, priority: usize, work: Box<dyn GenericWork<VM>>) {
        let _guard = self.monitor.0.lock().unwrap();
        self.monitor.1.notify_all();
        self.queue.write().unwrap().push(PrioritizedWork { priority, work });
    }
    pub fn add_with_highest_priority(&self, work: Box<dyn GenericWork<VM>>) -> usize {
        let priority = usize::max_value();
        self.add(priority, work);
        priority
    }
    /// Get a work packet (with the greatest priority) from this bucket
    fn poll(&self) -> Option<Box<dyn GenericWork<VM>>> {
        if !self.active.load(Ordering::SeqCst) { return None }
        self.queue.write().unwrap().pop().map(|v| v.work)
    }
}

pub struct Scheduler<VM: VMBinding> {
    /// Works that are scheduable at any time
    default_bucket: WorkBucket<VM>,
    /// Works that are scheduable within Stop-the-world
    stw_bucket: WorkBucket<VM>,
    /// workers
    worker_group: Option<Arc<WorkerGroup<VM>>>,
    /// Condition Variable
    pub monitor: Arc<(Mutex<()>, Condvar)>,
}

impl <VM: VMBinding> Scheduler<VM> {
    pub fn new() -> Arc<Self> {
        let monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        Arc::new(Self {
            default_bucket: WorkBucket::new(true, monitor.clone()), // `default_bucket` is always activated
            stw_bucket: WorkBucket::new(false, monitor.clone()),
            worker_group: None,
            monitor,
        })
    }

    pub fn initialize(&mut self, mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
        let size = 1;//mmtk.options.threads;

        self.worker_group = Some(WorkerGroup::new(size, Arc::downgrade(&mmtk.scheduler)));
        self.worker_group.as_ref().unwrap().spawn_workers(tls);
    }

    pub fn add<W: Work<VM=VM>>(&self, priority: usize, work: W) {
        if W::REQUIRES_STOP_THE_WORLD {
            self.stw_bucket.add(priority, box work);
        } else {
            self.default_bucket.add(priority, box work);
        }
    }

    pub fn add_with_highest_priority<W: Work<VM=VM>>(&self, work: W) -> usize {
        if W::REQUIRES_STOP_THE_WORLD {
            self.stw_bucket.add_with_highest_priority(box work)
        } else {
            self.default_bucket.add_with_highest_priority(box work)
        }
    }

    pub fn mutators_stopped(&self) {
        println!("mutators_stopped");
        self.stw_bucket.activate()
    }

    fn all_buckets_empty(&self) -> bool {
        self.default_bucket.is_empty() && self.stw_bucket.is_empty()
    }

    fn all_workers_packed(&self) -> bool {
        self.worker_group.as_ref().unwrap().workers.iter().all(|w| w.is_parked())
    }

    pub fn wait_for_completion(&self) {
        let mut guard = self.monitor.0.lock().unwrap();
        while !self.all_buckets_empty() || !self.all_workers_packed() {
            guard = self.monitor.1.wait(guard).unwrap();
        }
        self.stw_bucket.deactivate()
    }

    fn pop_scheduable_work(&self, worker: &Worker<VM>) -> Option<Box<dyn GenericWork<VM>>> {
        if let Some(work) = worker.local_works.poll() {
            return Some(work);
        }
        if let Some(work) = self.default_bucket.poll() {
            return Some(work);
        }
        if let Some(work) = self.stw_bucket.poll() {
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
            self.monitor.1.notify_all();
            worker.parked.store(true, Ordering::SeqCst);
            // Wait
            guard = self.monitor.1.wait(guard).unwrap();
            // Unpark this worker
            worker.parked.store(false, Ordering::SeqCst);
        }
    }
}
