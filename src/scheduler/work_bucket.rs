use super::work::Work;
use super::*;
use enum_map::Enum;
use spin::RwLock;
use std::cmp;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// A unique work-packet id for each instance of work-packet
#[derive(Eq, PartialEq, Clone, Copy)]
struct WorkUID(u64);

impl WorkUID {
    pub fn new() -> Self {
        static WORK_UID: AtomicU64 = AtomicU64::new(0);
        Self(WORK_UID.fetch_add(1, Ordering::Relaxed))
    }
}

struct PrioritizedWork<C: Context> {
    priority: usize,
    work_uid: WorkUID,
    work: Box<dyn Work<C>>,
}

impl<C: Context> PrioritizedWork<C> {
    pub fn new(priority: usize, work: Box<dyn Work<C>>) -> Self {
        Self {
            priority,
            work,
            work_uid: WorkUID::new(),
        }
    }
}

impl<C: Context> PartialEq for PrioritizedWork<C> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.work_uid == other.work_uid
    }
}

impl<C: Context> Eq for PrioritizedWork<C> {}

impl<C: Context> Ord for PrioritizedWork<C> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl<C: Context> PartialOrd for PrioritizedWork<C> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct WorkBucket<C: Context> {
    active: AtomicBool,
    /// A priority queue
    queue: RwLock<BinaryHeap<PrioritizedWork<C>>>,
    monitor: Arc<(Mutex<()>, Condvar)>,
    can_open: Option<Box<dyn Fn() -> bool>>,
}

unsafe impl<C: Context> Send for WorkBucket<C> {}
unsafe impl<C: Context> Sync for WorkBucket<C> {}

impl<C: Context> WorkBucket<C> {
    pub fn new(active: bool, monitor: Arc<(Mutex<()>, Condvar)>) -> Self {
        Self {
            active: AtomicBool::new(active),
            queue: Default::default(),
            monitor,
            can_open: None,
        }
    }
    fn notify_one_worker(&self) {
        let _guard = self.monitor.0.lock().unwrap();
        self.monitor.1.notify_one()
    }
    fn notify_all_workers(&self) {
        let _guard = self.monitor.0.lock().unwrap();
        self.monitor.1.notify_all()
    }
    pub fn is_activated(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    /// Enable the bucket
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }
    /// Test if the bucket is drained
    pub fn is_empty(&self) -> bool {
        self.queue.read().len() == 0
    }
    pub fn is_drained(&self) -> bool {
        self.is_activated() && self.is_empty()
    }
    /// Disable the bucket
    pub fn deactivate(&self) {
        debug_assert!(
            self.queue.read().is_empty(),
            "Bucket not drained before close"
        );
        self.active.store(false, Ordering::SeqCst);
    }
    /// Add a work packet to this bucket, with a given priority
    pub fn add_with_priority(&self, priority: usize, work: Box<dyn Work<C>>) {
        self.queue
            .write()
            .push(PrioritizedWork::new(priority, work));
        self.notify_one_worker(); // FIXME: Performance
    }
    /// Add a work packet to this bucket, with a default priority (1000)
    pub fn add<W: Work<C>>(&self, work: W) {
        self.add_with_priority(1000, box work);
    }
    pub fn bulk_add(&self, priority: usize, work_vec: Vec<Box<dyn Work<C>>>) {
        {
            let mut queue = self.queue.write();
            for w in work_vec {
                queue.push(PrioritizedWork::new(priority, w));
            }
        }
        self.notify_all_workers(); // FIXME: Performance
    }
    /// Get a work packet (with the greatest priority) from this bucket
    pub fn poll(&self) -> Option<Box<dyn Work<C>>> {
        if !self.active.load(Ordering::SeqCst) {
            return None;
        }
        self.queue.write().pop().map(|v| v.work)
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

#[derive(Debug, Enum, Copy, Clone, Eq, PartialEq)]
pub enum WorkBucketStage {
    Unconstrained,
    Prepare,
    Closure,
    // TODO: We only support final reference at the moment. If we have references of multiple strengths,
    // we may need more than one buckets for each strength.
    RefClosure,
    RefForwarding,
    Release,
    Final,
}
