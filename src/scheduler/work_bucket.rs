use super::worker::WorkerGroup;
use super::*;
use crate::vm::VMBinding;
use crossbeam::deque::{Injector, Steal, Worker};
use enum_map::Enum;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

struct BucketQueue<VM: VMBinding> {
    queue: Injector<Box<dyn GCWork<VM>>>,
}

impl<VM: VMBinding> BucketQueue<VM> {
    fn new() -> Self {
        Self {
            queue: Injector::new(),
        }
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    #[inline(always)]
    fn steal_batch_and_pop(
        &self,
        dest: &Worker<Box<dyn GCWork<VM>>>,
    ) -> Steal<Box<dyn GCWork<VM>>> {
        self.queue.steal_batch_and_pop(dest)
    }

    #[inline(always)]
    fn push(&self, w: Box<dyn GCWork<VM>>) {
        self.queue.push(w);
    }

    #[inline(always)]
    fn push_all(&self, ws: Vec<Box<dyn GCWork<VM>>>) {
        for w in ws {
            self.queue.push(w);
        }
    }
}

pub struct WorkBucket<VM: VMBinding> {
    active: AtomicBool,
    queue: BucketQueue<VM>,
    prioritized_queue: Option<BucketQueue<VM>>,
    monitor: Arc<(Mutex<()>, Condvar)>,
    can_open: Option<Box<dyn (Fn(&GCWorkScheduler<VM>) -> bool) + Send>>,
    group: Arc<WorkerGroup<VM>>,
}

impl<VM: VMBinding> WorkBucket<VM> {
    pub fn new(
        active: bool,
        monitor: Arc<(Mutex<()>, Condvar)>,
        group: Arc<WorkerGroup<VM>>,
    ) -> Self {
        Self {
            active: AtomicBool::new(active),
            queue: BucketQueue::new(),
            prioritized_queue: None,
            monitor,
            can_open: None,
            group,
        }
    }

    #[inline(always)]
    fn notify_one_worker(&self) {
        // If the bucket is not activated, don't notify anyone.
        if !self.is_activated() {
            return;
        }
        // Notify one if there're any parked workers.
        if self.group.parked_workers() > 0 {
            let _guard = self.monitor.0.lock().unwrap();
            self.monitor.1.notify_one()
        }
    }

    #[inline(always)]
    pub fn notify_all_workers(&self) {
        // If the bucket is not activated, don't notify anyone.
        if !self.is_activated() {
            return;
        }
        // Notify all if there're any parked workers.
        if self.group.parked_workers() > 0 {
            let _guard = self.monitor.0.lock().unwrap();
            self.monitor.1.notify_all()
        }
    }

    #[inline(always)]
    pub fn is_activated(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Enable the bucket
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    /// Test if the bucket is drained
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
            && self
                .prioritized_queue
                .as_ref()
                .map(|q| q.is_empty())
                .unwrap_or(true)
    }

    #[inline(always)]
    pub fn is_drained(&self) -> bool {
        self.is_activated() && self.is_empty()
    }

    /// Disable the bucket
    pub fn deactivate(&self) {
        debug_assert!(self.queue.is_empty(), "Bucket not drained before close");
        self.active.store(false, Ordering::Relaxed);
    }

    /// Add a work packet to this bucket
    /// Panic if this bucket cannot receive prioritized packets.
    #[inline(always)]
    pub fn add_prioritized(&self, work: Box<dyn GCWork<VM>>) {
        self.prioritized_queue.as_ref().unwrap().push(work);
        self.notify_one_worker();
    }

    /// Add a work packet to this bucket
    #[inline(always)]
    pub fn add<W: GCWork<VM>>(&self, work: W) {
        self.queue.push(Box::new(work));
        self.notify_one_worker();
    }

    /// Add a work packet to this bucket
    #[inline(always)]
    pub fn add_boxed(&self, work: Box<dyn GCWork<VM>>) {
        self.queue.push(work);
        self.notify_one_worker();
    }

    /// Add multiple packets with a higher priority.
    /// Panic if this bucket cannot receive prioritized packets.
    #[inline(always)]
    pub fn bulk_add_prioritized(&self, work_vec: Vec<Box<dyn GCWork<VM>>>) {
        self.prioritized_queue.as_ref().unwrap().push_all(work_vec);
        if self.is_activated() {
            self.notify_all_workers();
        }
    }

    /// Add multiple packets
    #[inline(always)]
    pub fn bulk_add(&self, work_vec: Vec<Box<dyn GCWork<VM>>>) {
        if work_vec.is_empty() {
            return;
        }
        self.queue.push_all(work_vec);
        if self.is_activated() {
            self.notify_all_workers();
        }
    }

    /// Get a work packet from this bucket
    #[inline(always)]
    pub fn poll(&self, worker: &Worker<Box<dyn GCWork<VM>>>) -> Steal<Box<dyn GCWork<VM>>> {
        if !self.is_activated() || self.is_empty() {
            return Steal::Empty;
        }
        if let Some(prioritized_queue) = self.prioritized_queue.as_ref() {
            prioritized_queue
                .steal_batch_and_pop(worker)
                .or_else(|| self.queue.steal_batch_and_pop(worker))
        } else {
            self.queue.steal_batch_and_pop(worker)
        }
    }

    pub fn set_open_condition(
        &mut self,
        pred: impl Fn(&GCWorkScheduler<VM>) -> bool + Send + 'static,
    ) {
        self.can_open = Some(Box::new(pred));
    }

    #[inline(always)]
    pub fn update(&self, scheduler: &GCWorkScheduler<VM>) -> bool {
        if let Some(can_open) = self.can_open.as_ref() {
            if !self.is_activated() && can_open(scheduler) {
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
    ClosureImmovable,
    Closure,
    SoftRefClosure,
    WeakRefClosure,
    FinalRefClosure,
    PhantomRefClosure,
    CalculateForwarding,
    SecondRoots,
    RefForwarding,
    FinalizableForwarding,
    Compact,
    Release,
    Final,
}

impl WorkBucketStage {
    #[inline]
    pub fn first_stw_stage() -> Self {
        WorkBucketStage::from_usize(1)
    }
}

pub const LAST_CLOSURE_BUCKET: WorkBucketStage = WorkBucketStage::PhantomRefClosure;
