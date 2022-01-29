use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, GCWorkerShared, WorkerGroup};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::vm::Collection;
use crate::vm::{GCThreadContext, VMBinding};
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex, RwLock};

pub enum CoordinatorMessage<VM: VMBinding> {
    Work(Box<dyn CoordinatorWork<VM>>),
    AllWorkerParked,
    BucketDrained,
}

pub struct GCWorkScheduler<VM: VMBinding> {
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Work for the coordinator thread
    pub coordinator_work: WorkBucket<VM>,
    /// workers
    worker_group: Option<WorkerGroup<VM>>,
    /// Condition Variable for worker synchronization
    pub worker_monitor: Arc<(Mutex<()>, Condvar)>,
    coordinator_worker_shared: Option<RwLock<Arc<GCWorkerShared<VM>>>>,
    /// A message channel to send new coordinator work and other actions to the coordinator thread
    pub startup: Mutex<Option<Box<dyn CoordinatorWork<VM>>>>,
    pub finalizer: Mutex<Option<Box<dyn CoordinatorWork<VM>>>>,
    /// A callback to be fired after the `Closure` bucket is drained.
    /// This callback should return `true` if it adds more work packets to the
    /// `Closure` bucket. `WorkBucket::can_open` then consult this return value
    /// to prevent the GC from proceeding to the next stage, if we still have
    /// `Closure` work to do.
    ///
    /// We use this callback to process ephemeron objects. `closure_end` can re-enable
    /// the `Closure` bucket multiple times to iteratively discover and process
    /// more ephemeron objects.
    pub closure_end: Mutex<Option<Box<dyn Send + Fn() -> bool>>>,
}

// The 'channel' inside Scheduler disallows Sync for Scheduler. We have to make sure we use channel properly:
// 1. We should never directly use Sender. We clone the sender and let each worker have their own copy.
// 2. Only the coordinator can use Receiver.
// TODO: We should remove channel from Scheduler, and directly send Sender/Receiver when creating the coordinator and
// the workers.
unsafe impl<VM: VMBinding> Sync for GCWorkScheduler<VM> {}

impl<VM: VMBinding> GCWorkScheduler<VM> {
    pub fn new() -> Arc<Self> {
        let worker_monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        Arc::new(Self {
            work_buckets: enum_map! {
                WorkBucketStage::Unconstrained => WorkBucket::new(true, worker_monitor.clone()),
                WorkBucketStage::Prepare => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Closure => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::RefClosure => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::CalculateForwarding => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::RefForwarding => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Compact => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Release => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Final => WorkBucket::new(false, worker_monitor.clone()),
            },
            coordinator_work: WorkBucket::new(true, worker_monitor.clone()),
            worker_group: None,
            worker_monitor,
            coordinator_worker_shared: None,
            startup: Mutex::new(None),
            finalizer: Mutex::new(None),
            closure_end: Mutex::new(None),
        })
    }

    #[inline]
    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().unwrap().worker_count()
    }

    pub fn initialize(
        self: &'static Arc<Self>,
        num_workers: usize,
        mmtk: &'static MMTK<VM>,
        tls: VMThread,
    ) {
        use crate::scheduler::work_bucket::WorkBucketStage::*;
        let num_workers = if cfg!(feature = "single_worker") {
            1
        } else {
            num_workers
        };

        let (sender, receiver) = channel::<CoordinatorMessage<VM>>();

        let mut self_mut = self.clone();
        let self_mut = unsafe { Arc::get_mut_unchecked(&mut self_mut) };

        let coordinator_worker = GCWorker::new(mmtk, 0, self.clone(), true, sender.clone());
        self_mut.coordinator_worker_shared = Some(RwLock::new(coordinator_worker.shared.clone()));

        let (worker_group, spawn_workers) =
            WorkerGroup::new(mmtk, num_workers, self.clone(), sender);
        self_mut.worker_group = Some(worker_group);

        {
            // Unconstrained is always open. Prepare will be opened at the beginning of a GC.
            // This vec will grow for each stage we call with open_next()
            let mut open_stages: Vec<WorkBucketStage> = vec![Unconstrained, Prepare];
            // The rest will open after the previous stage is done.
            let mut open_next = |s: WorkBucketStage| {
                let cur_stages = open_stages.clone();
                self_mut.work_buckets[s].set_open_condition(move || {
                    let should_open =
                        self.are_buckets_drained(&cur_stages) && self.worker_group().all_parked();
                    // Additional check before the `RefClosure` bucket opens.
                    if should_open && s == WorkBucketStage::RefClosure {
                        if let Some(closure_end) = self.closure_end.lock().unwrap().as_ref() {
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
            open_next(RefClosure);
            open_next(CalculateForwarding);
            open_next(RefForwarding);
            open_next(Compact);
            open_next(Release);
            open_next(Final);
        }

        // Now that the scheduler is initialized, we spawn the worker threads and the controller thread.
        spawn_workers(tls);

        let gc_controller = GCController::new(
            mmtk,
            mmtk.plan.base().control_collector_context.clone(),
            self.clone(),
            receiver,
            coordinator_worker,
        );

        VM::VMCollection::spawn_gc_thread(tls, GCThreadContext::<VM>::Controller(gc_controller));
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
        self.work_buckets[WorkBucketStage::RefClosure]
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

        // Finalization
        if !*plan.base().options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            self.work_buckets[WorkBucketStage::RefClosure]
                .add(Finalization::<C::ProcessEdgesWorkType>::new());
            // forward refs
            if plan.constraints().needs_forward_after_liveness {
                self.work_buckets[WorkBucketStage::RefForwarding]
                    .add(ForwardFinalization::<C::ProcessEdgesWorkType>::new());
            }
        }

        // Set EndOfGC to run at the end
        self.set_finalizer(Some(EndOfGC));
    }

    fn are_buckets_drained(&self, buckets: &[WorkBucketStage]) -> bool {
        buckets.iter().all(|&b| self.work_buckets[b].is_drained())
    }

    pub fn set_initializer<W: CoordinatorWork<VM>>(&self, w: Option<W>) {
        *self.startup.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<VM>>);
    }

    pub fn take_initializer(&self) -> Option<Box<dyn CoordinatorWork<VM>>> {
        self.startup.lock().unwrap().take()
    }

    pub fn set_finalizer<W: CoordinatorWork<VM>>(&self, w: Option<W>) {
        *self.finalizer.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<VM>>);
    }

    pub fn take_finalizer(&self) -> Option<Box<dyn CoordinatorWork<VM>>> {
        self.finalizer.lock().unwrap().take()
    }

    pub fn on_closure_end(&self, f: Box<dyn Send + Fn() -> bool>) {
        *self.closure_end.lock().unwrap() = Some(f);
    }

    pub fn worker_group(&self) -> &WorkerGroup<VM> {
        self.worker_group.as_ref().unwrap()
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
            buckets_updated |= bucket.update();
        }
        if buckets_updated {
            // Notify the workers for new work
            let _guard = self.worker_monitor.0.lock().unwrap();
            self.worker_monitor.1.notify_all();
        }
    }

    pub fn deactivate_all(&self) {
        self.work_buckets[WorkBucketStage::Prepare].deactivate();
        self.work_buckets[WorkBucketStage::Closure].deactivate();
        self.work_buckets[WorkBucketStage::RefClosure].deactivate();
        self.work_buckets[WorkBucketStage::CalculateForwarding].deactivate();
        self.work_buckets[WorkBucketStage::RefForwarding].deactivate();
        self.work_buckets[WorkBucketStage::Compact].deactivate();
        self.work_buckets[WorkBucketStage::Release].deactivate();
        self.work_buckets[WorkBucketStage::Final].deactivate();
    }

    pub fn reset_state(&self) {
        // self.work_buckets[WorkBucketStage::Prepare].deactivate();
        self.work_buckets[WorkBucketStage::Closure].deactivate();
        self.work_buckets[WorkBucketStage::RefClosure].deactivate();
        self.work_buckets[WorkBucketStage::CalculateForwarding].deactivate();
        self.work_buckets[WorkBucketStage::RefForwarding].deactivate();
        self.work_buckets[WorkBucketStage::Compact].deactivate();
        self.work_buckets[WorkBucketStage::Release].deactivate();
        self.work_buckets[WorkBucketStage::Final].deactivate();
    }

    pub fn debug_assert_all_buckets_deactivated(&self) {
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Closure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::CalculateForwarding].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefForwarding].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Compact].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Release].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Final].is_activated());
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
            if self.worker_group().all_parked() {
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
        for worker in &self.worker_group().workers_shared {
            let worker_stat = worker.borrow_stat();
            worker_stat.enable();
        }
        let coordinator_worker_shared = self
            .coordinator_worker_shared
            .as_ref()
            .unwrap()
            .read()
            .unwrap();
        let coordinator_worker_stat = coordinator_worker_shared.borrow_stat();
        coordinator_worker_stat.enable();
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.worker_group().workers_shared {
            let worker_stat = worker.borrow_stat();
            summary.merge(&worker_stat);
        }
        let coordinator_worker_shared = self
            .coordinator_worker_shared
            .as_ref()
            .unwrap()
            .read()
            .unwrap();
        let coordinator_worker_stat = coordinator_worker_shared.borrow_stat();
        summary.merge(&coordinator_worker_stat);
        summary.harness_stat()
    }

    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().control_collector_context.clear_request();
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        self.work_buckets[WorkBucketStage::Prepare].activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
