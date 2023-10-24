use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, GCWorkerShared, ThreadId, WorkerGroup, WorkerMonitor};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::util::options::AffinityKind;
use crate::util::rust_util::array_from_fn;
use crate::vm::Collection;
use crate::vm::{GCThreadContext, VMBinding};
use crossbeam::deque::{self, Steal};
use enum_map::{Enum, EnumMap};
use std::collections::HashMap;
use std::sync::Arc;

pub struct GCWorkScheduler<VM: VMBinding> {
    /// Work buckets
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Workers
    pub(crate) worker_group: Arc<WorkerGroup<VM>>,
    /// The shared part of the GC worker object of the controller thread
    coordinator_worker_shared: Arc<GCWorkerShared<VM>>,
    /// Condition Variable for worker synchronization
    pub(crate) worker_monitor: Arc<WorkerMonitor>,
    /// How to assign the affinity of each GC thread. Specified by the user.
    affinity: AffinityKind,
}

// FIXME: GCWorkScheduler should be naturally Sync, but we cannot remove this `impl` yet.
// Some subtle interaction between ObjectRememberingBarrier, Mutator and some GCWork instances
// makes the compiler think WorkBucket is not Sync.
unsafe impl<VM: VMBinding> Sync for GCWorkScheduler<VM> {}

impl<VM: VMBinding> GCWorkScheduler<VM> {
    pub fn new(num_workers: usize, affinity: AffinityKind) -> Arc<Self> {
        let worker_monitor: Arc<WorkerMonitor> = Arc::new(WorkerMonitor::new(num_workers));
        let worker_group = WorkerGroup::new(num_workers);

        // Create work buckets for workers.
        // TODO: Replace `array_from_fn` with `std::array::from_fn` after bumping MSRV.
        let mut work_buckets = EnumMap::from_array(array_from_fn(|stage_num| {
            let stage = WorkBucketStage::from_usize(stage_num);
            let active = stage == WorkBucketStage::Unconstrained;
            WorkBucket::new(active, worker_monitor.clone())
        }));

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
            affinity,
        })
    }

    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().worker_count()
    }

    /// Create GC threads, including the controller thread and all workers.
    pub fn spawn_gc_threads(self: &Arc<Self>, mmtk: &'static MMTK<VM>, tls: VMThread) {
        // Spawn the controller thread.
        let coordinator_worker = GCWorker::new(
            mmtk,
            usize::MAX,
            self.clone(),
            true,
            self.coordinator_worker_shared.clone(),
            deque::Worker::new_fifo(),
        );
        let gc_controller = GCController::new(
            mmtk,
            mmtk.gc_requester.clone(),
            self.clone(),
            coordinator_worker,
        );
        VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Controller(gc_controller));

        self.worker_group.spawn(mmtk, tls)
    }

    /// Resolve the affinity of a thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        self.affinity.resolve_affinity(thread);
    }

    /// Schedule all the common work packets
    pub fn schedule_common_work<C: GCWorkContext<VM = VM>>(&self, plan: &'static C::PlanType) {
        use crate::plan::Plan;
        use crate::scheduler::gc_work::*;
        // Stop & scan mutators (mutator scanning can happen before STW)
        self.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<C>::new());

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
        buckets.iter().all(|&b| self.work_buckets[b].is_drained())
    }

    pub fn all_buckets_empty(&self) -> bool {
        self.work_buckets.values().all(|bucket| bucket.is_empty())
    }

    /// Schedule "sentinel" work packets for all activated buckets.
    pub(crate) fn schedule_sentinels(&self) -> bool {
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
    pub(crate) fn update_buckets(&self) -> bool {
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
                probe!(mmtk, bucket_opened, id);
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

    /// Check if all the work buckets are empty
    pub(crate) fn assert_all_activated_buckets_are_empty(&self) {
        let mut error_example = None;
        for (id, bucket) in self.work_buckets.iter() {
            if bucket.is_activated() && !bucket.is_empty() {
                error!("Work bucket {:?} is active but not empty!", id);
                // This error can be hard to reproduce.
                // If an error happens in the release build where logs are turned off,
                // we should show at least one abnormal bucket in the panic message
                // so that we still have some information for debugging.
                error_example = Some(id);
            }
        }
        if let Some(id) = error_example {
            panic!("Some active buckets (such as {:?}) are not empty.", id);
        }
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
        loop {
            // Retry polling
            if let Some(work) = self.poll_schedulable_work(worker) {
                return work;
            }

            self.worker_monitor.park_and_wait(worker);
        }
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
        mmtk.gc_requester.clear_request();
        let first_stw_bucket = &self.work_buckets[WorkBucketStage::first_stw_stage()];
        debug_assert!(!first_stw_bucket.is_activated());
        // Note: This is the only place where a non-coordinator thread opens a bucket.
        // If the `StopMutators` is executed by the coordinator thread, it will open
        // the `Prepare` bucket and let workers start executing packets while the coordinator
        // can still add more work packets to `Prepare`.  However, since `Prepare` is the first STW
        // bucket and only the coordinator can open any subsequent buckets, workers cannot execute
        // work packets out of order.  This is not generally true if we are not opening the first
        // STW bucket.  In the future, we should redesign the opening condition of work buckets to
        // make the synchronization more robust,
        first_stw_bucket.activate();
        self.worker_monitor.notify_work_available(true);
    }
}
