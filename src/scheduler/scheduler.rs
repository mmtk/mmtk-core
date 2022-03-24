use super::stat::SchedulerStat;
use super::work_bucket::WorkBucketStage::*;
use super::work_bucket::*;
use super::worker::{GCWorker, GCWorkerShared};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::vm::Collection;
use crate::vm::{GCThreadContext, VMBinding};
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex};

pub enum CoordinatorMessage<VM: VMBinding> {
    Work(Box<dyn CoordinatorWork<VM>>),
    AllWorkerParked,
    BucketDrained,
}

/// The shared data structure for distributing work packets between worker threads and the coordinator thread.
pub struct GCWorkScheduler<VM: VMBinding> {
    /// Work buckets for worker threads
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Work for the coordinator thread
    pub coordinator_work: WorkBucket<VM>,
    /// The shared parts of GC workers
    pub workers_shared: Vec<Arc<GCWorkerShared<VM>>>,
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
            let mut open_stages: Vec<WorkBucketStage> = vec![Unconstrained, Prepare];
            // The rest will open after the previous stage is done.
            let mut open_next = |s: WorkBucketStage| {
                let cur_stages = open_stages.clone();
                work_buckets[s].set_open_condition(move |scheduler: &GCWorkScheduler<VM>| {
                    let should_open = scheduler.are_buckets_drained(&cur_stages)
                        && scheduler.all_workers_parked();
                    // Additional check before the `RefClosure` bucket opens.
                    if should_open && s == crate::scheduler::work_bucket::LAST_CLOSURE_BUCKET {
                        if let Some(closure_end) = scheduler.closure_end.lock().unwrap().as_ref() {
                            if closure_end() {
                                // Don't open `RefClosure` if `closure_end` added more works to `Closure`.
                                return false;
                            }
                        }
                    }
                    should_open
                });
                open_stages.push(s);
            };

            open_next(Closure);
            open_next(SoftRefClosure);
            open_next(WeakRefClosure);
            open_next(FinalRefClosure);
            open_next(PhantomRefClosure);
            open_next(CalculateForwarding);
            open_next(SecondRoots);
            open_next(RefForwarding);
            open_next(FinalizableForwarding);
            open_next(Compact);
            open_next(Release);
            open_next(Final);
        }

        // Create the work bucket for the controller.
        let coordinator_work = WorkBucket::new(true, worker_monitor.clone());

        // We prepare the shared part of workers, but do not create the actual workers now.
        // The shared parts of workers are communication hubs between controller and workers.
        let workers_shared = (0..num_workers)
            .map(|_| Arc::new(GCWorkerShared::<VM>::new(worker_monitor.clone())))
            .collect::<Vec<_>>();

        // Similarly, we create the shared part of the work of the controller, but not the controller itself.
        let coordinator_worker_shared = Arc::new(GCWorkerShared::<VM>::new(worker_monitor.clone()));

        Arc::new(Self {
            work_buckets,
            coordinator_work,
            workers_shared,
            coordinator_worker_shared,
            worker_monitor,
            closure_end: Mutex::new(None),
        })
    }

    #[inline]
    pub fn num_workers(&self) -> usize {
        self.workers_shared.len()
    }

    pub fn all_workers_parked(&self) -> bool {
        self.workers_shared.iter().all(|w| w.is_parked())
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

        // Spawn each worker thread.
        for (ordinal, shared) in self.workers_shared.iter().enumerate() {
            let worker = Box::new(GCWorker::new(
                mmtk,
                ordinal,
                self.clone(),
                false,
                sender.clone(),
                shared.clone(),
            ));
            VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Worker(worker));
        }
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

        // VM-specific weak ref processing
        self.work_buckets[WorkBucketStage::WeakRefClosure]
            .add(ProcessWeakRefs::<C::ProcessEdgesWorkType>::new());

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

    /// Open buckets if their conditions are met
    pub fn update_buckets(&self) {
        let mut buckets_updated = false;
        for (id, bucket) in self.work_buckets.iter() {
            if id == WorkBucketStage::Unconstrained {
                continue;
            }
            buckets_updated |= bucket.update(self);
        }
        if buckets_updated {
            // Notify the workers for new work
            let _guard = self.worker_monitor.0.lock().unwrap();
            self.worker_monitor.1.notify_all();
        }
    }

    pub fn deactivate_all(&self) {
        for (stage, bucket) in self.work_buckets.iter() {
            if stage == WorkBucketStage::Unconstrained {
                continue;
            }

            bucket.deactivate();
        }
    }

    pub fn reset_state(&self) {
        for (stage, bucket) in self.work_buckets.iter() {
            if stage == WorkBucketStage::Unconstrained || stage == WorkBucketStage::Prepare {
                continue;
            }

            bucket.deactivate();
        }
    }

    pub fn debug_assert_all_buckets_deactivated(&self) {
        for (stage, bucket) in self.work_buckets.iter() {
            if stage == WorkBucketStage::Unconstrained {
                return;
            }

            debug_assert!(!bucket.is_activated());
        }
    }

    pub fn add_coordinator_work(&self, work: impl CoordinatorWork<VM>, worker: &GCWorker<VM>) {
        worker
            .sender
            .send(CoordinatorMessage::Work(box work))
            .unwrap();
    }

    #[inline]
    fn pop_scheduable_work(&self, worker: &GCWorker<VM>) -> Option<(Box<dyn GCWork<VM>>, bool)> {
        if let Some(work) = worker.shared.local_work_bucket.poll() {
            return Some((work, worker.shared.local_work_bucket.is_empty()));
        }
        for work_bucket in self.work_buckets.values() {
            if let Some(work) = work_bucket.poll() {
                return Some((work, work_bucket.is_empty()));
            }
        }
        None
    }

    /// Get a scheduable work. Called by workers
    #[inline]
    pub fn poll(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        let work = if let Some((work, bucket_is_empty)) = self.pop_scheduable_work(worker) {
            if bucket_is_empty {
                worker
                    .sender
                    .send(CoordinatorMessage::BucketDrained)
                    .unwrap();
            }
            work
        } else {
            self.poll_slow(worker)
        };
        work
    }

    #[cold]
    fn poll_slow(&self, worker: &GCWorker<VM>) -> Box<dyn GCWork<VM>> {
        debug_assert!(!worker.shared.is_parked());
        let mut guard = self.worker_monitor.0.lock().unwrap();
        loop {
            debug_assert!(!worker.shared.is_parked());
            if let Some((work, bucket_is_empty)) = self.pop_scheduable_work(worker) {
                if bucket_is_empty {
                    worker
                        .sender
                        .send(CoordinatorMessage::BucketDrained)
                        .unwrap();
                }
                return work;
            }
            // Park this worker
            worker.shared.parked.store(true, Ordering::SeqCst);
            if self.all_workers_parked() {
                worker
                    .sender
                    .send(CoordinatorMessage::AllWorkerParked)
                    .unwrap();
            }
            // Wait
            guard = self.worker_monitor.1.wait(guard).unwrap();
            // Unpark this worker
            worker.shared.parked.store(false, Ordering::SeqCst);
        }
    }

    pub fn enable_stat(&self) {
        for worker in &self.workers_shared {
            let worker_stat = worker.borrow_stat();
            worker_stat.enable();
        }
        let coordinator_worker_stat = self.coordinator_worker_shared.borrow_stat();
        coordinator_worker_stat.enable();
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.workers_shared {
            let worker_stat = worker.borrow_stat();
            summary.merge(&worker_stat);
        }
        let coordinator_worker_stat = self.coordinator_worker_shared.borrow_stat();
        summary.merge(&coordinator_worker_stat);
        summary.harness_stat()
    }

    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().gc_requester.clear_request();
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        self.work_buckets[WorkBucketStage::Prepare].activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
