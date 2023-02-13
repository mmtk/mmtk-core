use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, GCWorkerShared, ParkingGuard, ThreadId, WorkerGroup};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::util::options::AffinityKind;
use crate::vm::Collection;
use crate::vm::{GCThreadContext, VMBinding};
use crossbeam::deque::{self, Steal};
use enum_map::Enum;
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex};

pub enum CoordinatorMessage<VM: VMBinding> {
    /// Send a work-packet to the coordinator thread/
    Work(Box<dyn CoordinatorWork<VM>>),
    /// Notify the coordinator thread that all GC tasks are finished.
    /// When sending this message, all the work buckets should be
    /// empty, and all the workers should be parked.
    Finish,
}

pub struct GCWorkScheduler<VM: VMBinding> {
    /// Work buckets
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Workers
    pub worker_group: Arc<WorkerGroup<VM>>,
    /// The shared part of the GC worker object of the controller thread
    coordinator_worker_shared: Arc<GCWorkerShared<VM>>,
    /// Condition Variable for worker synchronization
    pub worker_monitor: Arc<(Mutex<()>, Condvar)>,
    /// Counter for pending coordinator messages.
    pub(super) pending_coordinator_packets: AtomicUsize,
    /// How to assign the affinity of each GC thread. Specified by the user.
    affinity: AffinityKind,
}

// FIXME: GCWorkScheduler should be naturally Sync, but we cannot remove this `impl` yet.
// Some subtle interaction between ObjectRememberingBarrier, Mutator and some GCWork instances
// makes the compiler think WorkBucket is not Sync.
unsafe impl<VM: VMBinding> Sync for GCWorkScheduler<VM> {}

impl<VM: VMBinding> GCWorkScheduler<VM> {
    pub fn new(num_workers: usize, affinity: AffinityKind) -> Arc<Self> {
        let worker_monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        let worker_group = WorkerGroup::new(num_workers);

        // Create work buckets for workers.
        let mut work_buckets = enum_map! {
            WorkBucketStage::Unconstrained => WorkBucket::new(true, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::Prepare => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::Closure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::SoftRefClosure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::WeakRefClosure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::FinalRefClosure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::PhantomRefClosure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::VMRefClosure => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::CalculateForwarding => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::SecondRoots => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::RefForwarding => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::FinalizableForwarding => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::VMRefForwarding => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::Compact => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::Release => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
            WorkBucketStage::Final => WorkBucket::new(false, worker_monitor.clone(), worker_group.clone()),
        };

        // Set the open condition of each bucket.
        {
            // Unconstrained is always open. Prepare will be opened at the beginning of a GC.
            // This vec will grow for each stage we call with open_next()
            let first_stw_stage = WorkBucketStage::first_stw_stage();
            let mut open_stages: Vec<WorkBucketStage> = vec![first_stw_stage];
            // The rest will open after the previous stage is done.
            let stages = (0..WorkBucketStage::LENGTH).map(WorkBucketStage::from_usize);
            for stage in stages {
                if stage != WorkBucketStage::Unconstrained && stage != first_stw_stage {
                    let cur_stages = open_stages.clone();
                    work_buckets[stage].set_open_condition(
                        move |scheduler: &GCWorkScheduler<VM>| {
                            scheduler.are_buckets_drained(&cur_stages)
                        },
                    );
                    open_stages.push(stage);
                }
            }
        }

        let coordinator_worker_shared = Arc::new(GCWorkerShared::<VM>::new(None));

        Arc::new(Self {
            work_buckets,
            worker_group,
            coordinator_worker_shared,
            worker_monitor,
            pending_coordinator_packets: AtomicUsize::new(0),
            affinity,
        })
    }

    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().worker_count()
    }

    /// Create GC threads, including the controller thread and all workers.
    pub fn spawn_gc_threads(self: &Arc<Self>, mmtk: &'static MMTK<VM>, tls: VMThread) {
        // Create the communication channel.
        let (sender, receiver) = channel::<CoordinatorMessage<VM>>();

        // Spawn the controller thread.
        let coordinator_worker = GCWorker::new(
            mmtk,
            usize::MAX,
            self.clone(),
            true,
            sender.clone(),
            self.coordinator_worker_shared.clone(),
            deque::Worker::new_fifo(),
        );
        let gc_controller = GCController::new(
            mmtk,
            mmtk.plan.base().gc_requester.clone(),
            self.clone(),
            receiver,
            coordinator_worker,
        );
        VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Controller(gc_controller));

        self.worker_group.spawn(mmtk, sender, tls)
    }

    /// Resolve the affinity of a thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        self.affinity.resolve_affinity(thread);
    }

    /// Schedule all the common work packets
    pub fn schedule_common_work<C: GCWorkContext<VM = VM> + 'static>(
        &self,
        plan: &'static C::PlanType,
    ) {
        use crate::plan::Plan;
        use crate::scheduler::gc_work::*;
        // Stop & scan mutators (mutator scanning can happen before STW)
        self.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<C::ProcessEdgesWorkType>::new());

        // Prepare global/collectors/mutators
        self.work_buckets[WorkBucketStage::Prepare].add(Prepare::<C>::new(plan));

        // Release global/collectors/mutators
        self.work_buckets[WorkBucketStage::Release].add(Release::<C>::new(plan));

        // Analysis GC work
        #[cfg(feature = "analysis")]
        {
            use crate::util::analysis::GcHookWork;
            self.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
        }

        // Sanity
        #[cfg(feature = "sanity")]
        {
            use crate::util::sanity::sanity_checker::ScheduleSanityGC;
            self.work_buckets[WorkBucketStage::Final]
                .add(ScheduleSanityGC::<C::PlanType>::new(plan));
        }

        // Reference processing
        if !*plan.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
            };
            self.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<C::ProcessEdgesWorkType>::new());
            self.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<C::ProcessEdgesWorkType>::new());
            self.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<C::ProcessEdgesWorkType>::new());

            use crate::util::reference_processor::RefForwarding;
            if plan.constraints().needs_forward_after_liveness {
                self.work_buckets[WorkBucketStage::RefForwarding]
                    .add(RefForwarding::<C::ProcessEdgesWorkType>::new());
            }

            use crate::util::reference_processor::RefEnqueue;
            self.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }

        // Finalization
        if !*plan.base().options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            self.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<C::ProcessEdgesWorkType>::new());
            // forward refs
            if plan.constraints().needs_forward_after_liveness {
                self.work_buckets[WorkBucketStage::FinalizableForwarding]
                    .add(ForwardFinalization::<C::ProcessEdgesWorkType>::new());
            }
        }

        // We add the VM-specific weak ref processing work regardless of MMTK-side options,
        // including Options::no_finalizer and Options::no_reference_types.
        //
        // VMs need weak reference handling to function properly.  The VM may treat weak references
        // as strong references, but it is not appropriate to simply disable weak reference
        // handling from MMTk's side.  The VM, however, may choose to do nothing in
        // `Collection::process_weak_refs` if appropriate.
        //
        // It is also not sound for MMTk core to turn off weak
        // reference processing or finalization alone, because (1) not all VMs have the notion of
        // weak references or finalizers, so it may not make sence, and (2) the VM may
        // processing them together.

        // VM-specific weak ref processing
        // The `VMProcessWeakRefs` work packet is set as the sentinel so that it is executed when
        // the `VMRefClosure` bucket is drained.  The VM binding may spawn new work packets into
        // the `VMRefClosure` bucket, and request another `VMProcessWeakRefs` work packet to be
        // executed again after this bucket is drained again.  Strictly speaking, the first
        // `VMProcessWeakRefs` packet can be an ordinary packet (doesn't have to be a sentinel)
        // because there are no other packets in the bucket.  We set it as sentinel for
        // consistency.
        self.work_buckets[WorkBucketStage::VMRefClosure]
            .set_sentinel(Box::new(VMProcessWeakRefs::<C::ProcessEdgesWorkType>::new()));

        if plan.constraints().needs_forward_after_liveness {
            // VM-specific weak ref forwarding
            self.work_buckets[WorkBucketStage::VMRefForwarding]
                .add(VMForwardWeakRefs::<C::ProcessEdgesWorkType>::new());
        }

        self.work_buckets[WorkBucketStage::Release].add(VMPostForwarding::<VM>::default());
    }

    fn are_buckets_drained(&self, buckets: &[WorkBucketStage]) -> bool {
        debug_assert!(
            self.pending_coordinator_packets.load(Ordering::SeqCst) == 0,
            "GCWorker attempted to open buckets when there are pending coordinator work packets"
        );
        buckets.iter().all(|&b| self.work_buckets[b].is_drained())
    }

    pub fn all_buckets_empty(&self) -> bool {
        self.work_buckets.values().all(|bucket| bucket.is_empty())
    }

    /// Schedule "sentinel" work packets for all activated buckets.
    fn schedule_sentinels(&self) -> bool {
        let mut new_packets = false;
        for (id, work_bucket) in self.work_buckets.iter() {
            if work_bucket.is_activated() && work_bucket.maybe_schedule_sentinel() {
                trace!("Scheduled sentinel packet into {:?}", id);
                new_packets = true;
            }
        }
        new_packets
    }

    /// Open buckets if their conditions are met.
    ///
    /// This function should only be called after all the workers are parked.
    /// No workers will be waked up by this function. The caller is responsible for that.
    ///
    /// Return true if there're any non-empty buckets updated.
    fn update_buckets(&self) -> bool {
        let mut buckets_updated = false;
        let mut new_packets = false;
        for i in 0..WorkBucketStage::LENGTH {
            let id = WorkBucketStage::from_usize(i);
            if id == WorkBucketStage::Unconstrained {
                continue;
            }
            let bucket = &self.work_buckets[id];
            let bucket_opened = bucket.update(self);
            buckets_updated = buckets_updated || bucket_opened;
            if bucket_opened {
                new_packets = new_packets || !bucket.is_drained();
                if new_packets {
                    // Quit the loop. There are already new packets in the newly opened buckets.
                    trace!("Found new packets at stage {:?}.  Break.", id);
                    break;
                }
                new_packets = new_packets || bucket.maybe_schedule_sentinel();
                if new_packets {
                    // Quit the loop. A sentinel packet is added to the newly opened buckets.
                    trace!("Sentinel is scheduled at stage {:?}.  Break.", id);
                    break;
                }
            }
        }
        buckets_updated && new_packets
    }

    pub fn deactivate_all(&self) {
        self.work_buckets.iter().for_each(|(id, bkt)| {
            if id != WorkBucketStage::Unconstrained {
                bkt.deactivate();
            }
        });
    }

    pub fn reset_state(&self) {
        let first_stw_stage = WorkBucketStage::first_stw_stage();
        self.work_buckets.iter().for_each(|(id, bkt)| {
            if id != WorkBucketStage::Unconstrained && id != first_stw_stage {
                bkt.deactivate();
            }
        });
    }

    pub fn debug_assert_all_buckets_deactivated(&self) {
        if cfg!(debug_assertions) {
            self.work_buckets.iter().for_each(|(id, bkt)| {
                if id != WorkBucketStage::Unconstrained {
                    assert!(!bkt.is_activated());
                }
            });
        }
    }

    pub fn add_coordinator_work(&self, work: impl CoordinatorWork<VM>, worker: &GCWorker<VM>) {
        self.pending_coordinator_packets
            .fetch_add(1, Ordering::SeqCst);
        worker
            .sender
            .send(CoordinatorMessage::Work(Box::new(work)))
            .unwrap();
    }

    /// Check if all the work buckets are empty
    fn all_activated_buckets_are_empty(&self) -> bool {
        for bucket in self.work_buckets.values() {
            if bucket.is_activated() && !bucket.is_drained() {
                return false;
            }
        }
        true
    }

    /// Get a schedulable work packet without retry.
    fn poll_schedulable_work_once(&self, worker: &GCWorker<VM>) -> Steal<Box<dyn GCWork<VM>>> {
        let mut should_retry = false;
        // Try find a packet that can be processed only by this worker.
        if let Some(w) = worker.shared.designated_work.pop() {
            return Steal::Success(w);
        }
        // Try get a packet from a work bucket.
        for work_bucket in self.work_buckets.values() {
            match work_bucket.poll(&worker.local_work_buffer) {
                Steal::Success(w) => return Steal::Success(w),
                Steal::Retry => should_retry = true,
                _ => {}
            }
        }
        // Try steal some packets from any worker
        for (id, worker_shared) in self.worker_group.workers_shared.iter().enumerate() {
            if id == worker.ordinal {
                continue;
            }
            match worker_shared.stealer.as_ref().unwrap().steal() {
                Steal::Success(w) => return Steal::Success(w),
                Steal::Retry => should_retry = true,
                _ => {}
            }
        }
        if should_retry {
            Steal::Retry
        } else {
            Steal::Empty
        }
    }

    /// Get a schedulable work packet.
    fn poll_schedulable_work(&self, worker: &GCWorker<VM>) -> Option<Box<dyn GCWork<VM>>> {
        // Loop until we successfully get a packet.
        loop {
            match self.poll_schedulable_work_once(worker) {
                Steal::Success(w) => {
                    return Some(w);
                }
                Steal::Retry => {
                    std::thread::yield_now();
                    continue;
                }
                Steal::Empty => {
                    return None;
                }
            }
        }
    }

    /// Called by workers to get a schedulable work packet.
    /// Park the worker if there're no available packets.
    pub fn poll(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        self.poll_schedulable_work(worker)
            .unwrap_or_else(|| self.poll_slow(worker))
    }

    fn poll_slow(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        // Note: The lock is released during `wait` in the loop.
        let mut guard = self.worker_monitor.0.lock().unwrap();
        'polling_loop: loop {
            // Retry polling
            if let Some(work) = self.poll_schedulable_work(worker) {
                return work;
            }
            // Prepare to park this worker
            let parking_guard = ParkingGuard::new(self.worker_group.as_ref());
            // If all workers are parked, try activate new buckets
            if parking_guard.all_parked() {
                // If there're any designated work, resume the workers and process them
                if self.worker_group.has_designated_work() {
                    assert!(
                        worker.shared.designated_work.is_empty(),
                        "The last parked worker has designated work."
                    );
                    self.worker_monitor.1.notify_all();
                    // The current worker is going to wait, because the designated work is not for it.
                } else if self.pending_coordinator_packets.load(Ordering::SeqCst) == 0 {
                    // See if any bucket has a sentinel.
                    if self.schedule_sentinels() {
                        // We're not going to sleep since new work packets are just scheduled.
                        break 'polling_loop;
                    }
                    // Try to open new buckets.
                    if self.update_buckets() {
                        // We're not going to sleep since a new bucket is just open.
                        break 'polling_loop;
                    }
                    debug_assert!(!self.worker_group.has_designated_work());
                    // The current pause is finished if we can't open more buckets.
                    worker.sender.send(CoordinatorMessage::Finish).unwrap();
                }
                // Otherwise, if there is still pending coordinator work, the last parked
                // worker will wait on the monitor, too.  The coordinator will notify a
                // worker (maybe not the current one) once it finishes executing all
                // coordinator work packets.
            }
            // Wait
            guard = self.worker_monitor.1.wait(guard).unwrap();
            // The worker is unparked here where `parking_guard` goes out of scope.
        }

        // We guarantee that we can at least fetch one packet when we reach here.
        let work = self.poll_schedulable_work(worker).unwrap();
        // Optimize for the case that a newly opened bucket only has one packet.
        // We only notify_all if there're more than one packets available.
        if !self.all_activated_buckets_are_empty() {
            // Have more jobs in this buckets. Notify other workers.
            self.worker_monitor.1.notify_all();
        }
        // Return this packet and execute it.
        work
    }

    pub fn enable_stat(&self) {
        for worker in &self.worker_group.workers_shared {
            let worker_stat = worker.borrow_stat();
            worker_stat.enable();
        }
        let coordinator_worker_stat = self.coordinator_worker_shared.borrow_stat();
        coordinator_worker_stat.enable();
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.worker_group.workers_shared {
            let worker_stat = worker.borrow_stat();
            summary.merge(&worker_stat);
        }
        let coordinator_worker_stat = self.coordinator_worker_shared.borrow_stat();
        summary.merge(&coordinator_worker_stat);
        summary.harness_stat()
    }

    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().gc_requester.clear_request();
        let first_stw_bucket = &self.work_buckets[WorkBucketStage::first_stw_stage()];
        debug_assert!(!first_stw_bucket.is_activated());
        first_stw_bucket.activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
