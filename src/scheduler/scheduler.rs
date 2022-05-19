use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, GCWorkerShared, WorkerGroup};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::vm::Collection;
use crate::vm::{GCThreadContext, VMBinding};
use crossbeam::deque::Steal;
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex};

pub enum CoordinatorMessage<VM: VMBinding> {
    Work(Box<dyn CoordinatorWork<VM>>),
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
    /// A callback to be fired after the `Closure` bucket is drained.
    /// This callback should return `true` if it adds more work packets to the
    /// `Closure` bucket. `WorkBucket::can_open` then consult this return value
    /// to prevent the GC from proceeding to the next stage, if we still have
    /// `Closure` work to do.
    ///
    /// We use this callback to process ephemeron objects. `closure_end` can re-enable
    /// the `Closure` bucket multiple times to iteratively discover and process
    /// more ephemeron objects.
    closure_end: Mutex<Option<Box<dyn Send + Fn() -> bool>>>,
}

// FIXME: GCWorkScheduler should be naturally Sync, but we cannot remove this `impl` yet.
// Some subtle interaction between ObjectRememberingBarrier, Mutator and some GCWork instances
// makes the compiler think WorkBucket is not Sync.
unsafe impl<VM: VMBinding> Sync for GCWorkScheduler<VM> {}

impl<VM: VMBinding> GCWorkScheduler<VM> {
    pub fn new(num_workers: usize) -> Arc<Self> {
        let worker_monitor: Arc<(Mutex<()>, Condvar)> = Default::default();

        // Create work buckets for workers.
        let mut work_buckets = enum_map! {
            WorkBucketStage::Unconstrained => WorkBucket::new(true, worker_monitor.clone()),
            WorkBucketStage::Prepare => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::Closure => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::SoftRefClosure => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::WeakRefClosure => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::FinalRefClosure => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::PhantomRefClosure => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::CalculateForwarding => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::SecondRoots => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::RefForwarding => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::FinalizableForwarding => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::Compact => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::Release => WorkBucket::new(false, worker_monitor.clone()),
            WorkBucketStage::Final => WorkBucket::new(false, worker_monitor.clone()),
        };

        // Set the open condition of each bucket.
        {
            // Unconstrained is always open. Prepare will be opened at the beginning of a GC.
            // This vec will grow for each stage we call with open_next()
            let first_stw_stage = work_buckets.iter().nth(1).map(|(id, _)| id).unwrap();
            let mut open_stages: Vec<WorkBucketStage> = vec![first_stw_stage];
            // The rest will open after the previous stage is done.
            let stages = work_buckets
                .iter()
                .map(|(stage, _)| stage)
                .collect::<Vec<_>>();
            let mut open_next = |s: WorkBucketStage| {
                let cur_stages = open_stages.clone();
                work_buckets[s].set_open_condition(move |scheduler: &GCWorkScheduler<VM>| {
                    let should_open = scheduler.are_buckets_drained(&cur_stages);
                    // Additional check before the `RefClosure` bucket opens.
                    if should_open && s == crate::scheduler::work_bucket::LAST_CLOSURE_BUCKET {
                        if let Some(closure_end) = scheduler.closure_end.lock().unwrap().as_ref() {
                            if closure_end() {
                                // Don't open `LAST_CLOSURE_BUCKET` if `closure_end` added more works to `Closure`.
                                return false;
                            }
                        }
                    }
                    should_open
                });
                open_stages.push(s);
            };

            for stages in stages {
                if stages != WorkBucketStage::Unconstrained && stages != first_stw_stage {
                    open_next(stages);
                }
            }
        }

        let coordinator_worker_shared = Arc::new(GCWorkerShared::<VM>::new());

        let worker_group = WorkerGroup::new(num_workers);
        work_buckets.values_mut().for_each(|bucket| {
            bucket.set_group(worker_group.clone());
        });

        Arc::new(Self {
            work_buckets,
            worker_group,
            coordinator_worker_shared,
            worker_monitor,
            closure_end: Mutex::new(None),
        })
    }

    #[inline]
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
            0,
            self.clone(),
            true,
            sender.clone(),
            self.coordinator_worker_shared.clone(),
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

            // VM-specific weak ref processing
            self.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(VMProcessWeakRefs::<C::ProcessEdgesWorkType>::new());

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
    }

    fn are_buckets_drained(&self, buckets: &[WorkBucketStage]) -> bool {
        buckets.iter().all(|&b| self.work_buckets[b].is_drained())
    }

    pub fn on_closure_end(&self, f: Box<dyn Send + Fn() -> bool>) {
        *self.closure_end.lock().unwrap() = Some(f);
    }

    pub fn all_buckets_empty(&self) -> bool {
        self.work_buckets.values().all(|bucket| bucket.is_empty())
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
        for (id, bucket) in self.work_buckets.iter() {
            if id == WorkBucketStage::Unconstrained {
                continue;
            }
            let bucket_opened = bucket.update(self);
            buckets_updated |= bucket_opened;
            if bucket_opened {
                new_packets |= !bucket.is_drained();
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
        let first_stw_stage = self.work_buckets.iter().nth(1).map(|(id, _)| id).unwrap();
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
        worker
            .sender
            .send(CoordinatorMessage::Work(Box::new(work)))
            .unwrap();
    }

    /// Check if all the work buckets are empty
    #[inline(always)]
    fn all_activated_buckets_are_empty(&self) -> bool {
        for bucket in self.work_buckets.values() {
            if bucket.is_activated() && !bucket.is_drained() {
                return false;
            }
        }
        true
    }

    /// Get a schedulable work packet without retry.
    #[inline(always)]
    fn poll_schedulable_work_once(&self, worker: &GCWorker<VM>) -> Steal<Box<dyn GCWork<VM>>> {
        let mut should_retry = false;
        // Try find a packet that can be processed only by this worker.
        if let Some(w) = worker.shared.local_work.pop() {
            return Steal::Success(w);
        }
        // Try get a packet from a work bucket.
        for work_bucket in self.work_buckets.values() {
            match work_bucket.poll(&worker.shared.local_work_buffer) {
                Steal::Success(w) => return Steal::Success(w),
                Steal::Retry => should_retry = true,
                _ => {}
            }
        }
        // Try steal some packets from any worker
        for (id, stealer) in self.worker_group.stealers.iter().enumerate() {
            if id == worker.ordinal {
                continue;
            }
            match stealer.steal() {
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
    #[inline]
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
    #[inline]
    pub fn poll(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        self.poll_schedulable_work(worker)
            .unwrap_or_else(|| self.poll_slow(worker))
    }

    #[cold]
    fn poll_slow(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        debug_assert!(!worker.shared.is_parked());
        let mut guard = self.worker_monitor.0.lock().unwrap();
        loop {
            debug_assert!(!worker.shared.is_parked());
            // Retry polling
            if let Some(work) = self.poll_schedulable_work(worker) {
                return work;
            }
            // Prepare to park this worker
            let all_parked = self.worker_group.inc_parked_workers();
            // If all workers are parked, try activate new buckets
            if all_parked {
                if self.update_buckets() {
                    // We're not going to sleep since a new bucket is just open.
                    self.worker_group.dec_parked_workers();
                    // We guarantee that we can at least fetch one packet.
                    let work = self.poll_schedulable_work(worker).unwrap();
                    // Optimize for the case that a newly opened bucket only has one packet.
                    // We only notify_all if there're motr than one packets available.
                    if !self.all_activated_buckets_are_empty() {
                        // Have more jobs in this buckets. Notify other workers.
                        self.worker_monitor.1.notify_all();
                    }
                    // Return this packet and execute it.
                    return work;
                }
                // The current pause is finished if if we can't open more buckets.
                worker.sender.send(CoordinatorMessage::Finish).unwrap();
            }
            // Wait
            guard = self.worker_monitor.1.wait(guard).unwrap();
            // Unpark this worker
            self.worker_group.dec_parked_workers();
            worker.shared.parked.store(false, Ordering::SeqCst);
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
        mmtk.plan.base().gc_requester.clear_request();
        let first_stw_bucket = self.work_buckets.values().nth(1).unwrap();
        debug_assert!(!first_stw_bucket.is_activated());
        first_stw_bucket.activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
