use super::worker::WorkerMonitor;
use super::*;
use crate::vm::VMBinding;
use crossbeam::deque::{Injector, Steal, Worker};
use enum_map::Enum;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct BucketQueue<VM: VMBinding> {
    queue: Injector<Box<dyn GCWork<VM>>>,
}

impl<VM: VMBinding> BucketQueue<VM> {
    fn new() -> Self {
        Self {
            queue: Injector::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn steal_batch_and_pop(
        &self,
        dest: &Worker<Box<dyn GCWork<VM>>>,
    ) -> Steal<Box<dyn GCWork<VM>>> {
        self.queue.steal_batch_and_pop(dest)
    }

    fn push(&self, w: Box<dyn GCWork<VM>>) {
        self.queue.push(w);
    }

    fn push_all(&self, ws: Vec<Box<dyn GCWork<VM>>>) {
        for w in ws {
            self.queue.push(w);
        }
    }
}

pub type BucketOpenCondition<VM> = Box<dyn (Fn(&GCWorkScheduler<VM>) -> bool) + Send>;

pub struct WorkBucket<VM: VMBinding> {
    active: AtomicBool,
    queue: BucketQueue<VM>,
    prioritized_queue: Option<BucketQueue<VM>>,
    monitor: Arc<WorkerMonitor>,
    can_open: Option<BucketOpenCondition<VM>>,
    /// After this bucket is activated and all pending work packets (including the packets in this
    /// bucket) are drained, this work packet, if exists, will be added to this bucket.  When this
    /// happens, it will prevent opening subsequent work packets.
    ///
    /// The sentinel work packet may set another work packet as the new sentinel which will be
    /// added to this bucket again after all pending work packets are drained.  This may happend
    /// again and again, causing the GC to stay at the same stage and drain work packets in a loop.
    ///
    /// This is useful for handling weak references that may expand the transitive closure
    /// recursively, such as ephemerons and Java-style SoftReference and finalizers.  Sentinels
    /// can be used repeatedly to discover and process more such objects.
    sentinel: Mutex<Option<Box<dyn GCWork<VM>>>>,
}

impl<VM: VMBinding> WorkBucket<VM> {
    pub(crate) fn new(active: bool, monitor: Arc<WorkerMonitor>) -> Self {
        Self {
            active: AtomicBool::new(active),
            queue: BucketQueue::new(),
            prioritized_queue: None,
            monitor,
            can_open: None,
            sentinel: Mutex::new(None),
        }
    }

    fn notify_one_worker(&self) {
        // If the bucket is not activated, don't notify anyone.
        if !self.is_activated() {
            return;
        }
        // Notify one if there're any parked workers.
        self.monitor.notify_work_available(false);
    }

    pub fn notify_all_workers(&self) {
        // If the bucket is not activated, don't notify anyone.
        if !self.is_activated() {
            return;
        }
        // Notify all if there're any parked workers.
        self.monitor.notify_work_available(true);
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
        self.queue.is_empty()
            && self
                .prioritized_queue
                .as_ref()
                .map(|q| q.is_empty())
                .unwrap_or(true)
    }

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
    pub fn add_prioritized(&self, work: Box<dyn GCWork<VM>>) {
        self.prioritized_queue.as_ref().unwrap().push(work);
        self.notify_one_worker();
    }

    /// Add a work packet to this bucket
    pub fn add<W: GCWork<VM>>(&self, work: W) {
        self.queue.push(Box::new(work));
        self.notify_one_worker();
    }

    /// Add a work packet to this bucket
    pub fn add_boxed(&self, work: Box<dyn GCWork<VM>>) {
        self.queue.push(work);
        self.notify_one_worker();
    }

    /// Add multiple packets with a higher priority.
    /// Panic if this bucket cannot receive prioritized packets.
    pub fn bulk_add_prioritized(&self, work_vec: Vec<Box<dyn GCWork<VM>>>) {
        self.prioritized_queue.as_ref().unwrap().push_all(work_vec);
        if self.is_activated() {
            self.notify_all_workers();
        }
    }

    /// Add multiple packets
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

    pub fn set_sentinel(&self, new_sentinel: Box<dyn GCWork<VM>>) {
        let mut sentinel = self.sentinel.lock().unwrap();
        *sentinel = Some(new_sentinel);
    }

    pub fn has_sentinel(&self) -> bool {
        let sentinel = self.sentinel.lock().unwrap();
        sentinel.is_some()
    }

    pub fn update(&self, scheduler: &GCWorkScheduler<VM>) -> bool {
        if let Some(can_open) = self.can_open.as_ref() {
            if !self.is_activated() && can_open(scheduler) {
                self.activate();
                return true;
            }
        }
        false
    }

    pub fn maybe_schedule_sentinel(&self) -> bool {
        debug_assert!(
            self.is_activated(),
            "Attempted to schedule sentinel work while bucket is not open"
        );
        let maybe_sentinel = {
            let mut sentinel = self.sentinel.lock().unwrap();
            sentinel.take()
        };
        if let Some(work) = maybe_sentinel {
            // We don't need to call `self.add` because this function is called by the coordinator
            // when workers are stopped.  We don't need to notify the workers because the
            // coordinator will do that later.
            // We can just "sneak" the sentinel work packet into the current bucket.
            self.queue.push(work);
            true
        } else {
            false
        }
    }
}

/// This enum defines all the work bucket types. The scheduler
/// will instantiate a work bucket for each stage defined here.
#[derive(Debug, Enum, Copy, Clone, Eq, PartialEq)]
pub enum WorkBucketStage {
    /// This bucket is always open.
    Unconstrained,
    /// Preparation work.  Plans, spaces, GC workers, mutators, etc. should be prepared for GC at
    /// this stage.
    Prepare,
    /// Clear the VO bit metadata.  Mainly used by ImmixSpace.
    #[cfg(feature = "vo_bit")]
    ClearVOBits,
    /// Compute the transtive closure starting from transitively pinning (TP) roots following only strong references.
    /// No objects in this closure are allow to move.
    TPinningClosure,
    /// Trace (non-transitively) pinning roots. Objects pointed by those roots must not move, but their children may. To ensure correctness, these must be processed after TPinningClosure
    PinningRootsTrace,
    /// Compute the transtive closure following only strong references.
    Closure,
    /// Handle Java-style soft references, and potentially expand the transitive closure.
    SoftRefClosure,
    /// Handle Java-style weak references.
    WeakRefClosure,
    /// Resurrect Java-style finalizable objects, and potentially expand the transitive closure.
    FinalRefClosure,
    /// Handle Java-style phantom references.
    PhantomRefClosure,
    /// Let the VM handle VM-specific weak data structures, including weak references, weak
    /// collections, table of finalizable objects, ephemerons, etc.  Potentially expand the
    /// transitive closure.
    ///
    /// NOTE: This stage is intended to replace the Java-specific weak reference handling stages
    /// above.
    VMRefClosure,
    /// Compute the forwarding addresses of objects (mark-compact-only).
    CalculateForwarding,
    /// Scan roots again to initiate another transitive closure to update roots and reference
    /// after computing the forwarding addresses (mark-compact-only).
    SecondRoots,
    /// Update Java-style weak references after computing forwarding addresses (mark-compact-only).
    ///
    /// NOTE: This stage should be updated to adapt to the VM-side reference handling.  It shall
    /// be kept after removing `{Soft,Weak,Final,Phantom}RefClosure`.
    RefForwarding,
    /// Update the list of Java-style finalization cadidates and finalizable objects after
    /// computing forwarding addresses (mark-compact-only).
    FinalizableForwarding,
    /// Let the VM handle the forwarding of reference fields in any VM-specific weak data
    /// structures, including weak references, weak collections, table of finalizable objects,
    /// ephemerons, etc., after computing forwarding addresses (mark-compact-only).
    ///
    /// NOTE: This stage is intended to replace Java-specific forwarding phases above.
    VMRefForwarding,
    /// Compact objects (mark-compact-only).
    Compact,
    /// Work packets that should be done just before GC shall go here.  This includes releasing
    /// resources and setting states in plans, spaces, GC workers, mutators, etc.
    Release,
    /// Resume mutators and end GC.
    Final,
}

impl WorkBucketStage {
    /// The first stop-the-world bucket.
    pub fn first_stw_stage() -> Self {
        WorkBucketStage::from_usize(1)
    }
}
