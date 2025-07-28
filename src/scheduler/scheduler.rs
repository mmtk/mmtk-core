use self::worker::PollResult;

use super::gc_work::ScheduleCollection;
use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, ThreadId, WorkerGroup};
use super::worker_goals::{WorkerGoal, WorkerGoals};
use super::worker_monitor::{LastParkedResult, WorkerMonitor};
use super::*;
use crate::global_state::GcStatus;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::util::options::AffinityKind;
use crate::vm::Collection;
use crate::vm::VMBinding;
use crate::Plan;
use crossbeam::deque::{Injector, Steal};
use enum_map::{Enum, EnumMap};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

type PostponeQueue<VM> = Injector<Box<dyn GCWork<VM>>>;

pub struct GCWorkScheduler<VM: VMBinding> {
    /// Work buckets
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Workers
    pub(crate) worker_group: Arc<WorkerGroup<VM>>,
    /// For synchronized communication between workers and with mutators.
    pub(crate) worker_monitor: Arc<WorkerMonitor>,
    /// How to assign the affinity of each GC thread. Specified by the user.
    affinity: AffinityKind,

    pub(super) postponed_concurrent_work:
        spin::RwLock<crossbeam::deque::Injector<Box<dyn GCWork<VM>>>>,
    pub(super) postponed_concurrent_work_prioritized:
        spin::RwLock<crossbeam::deque::Injector<Box<dyn GCWork<VM>>>>,
    in_gc_pause: std::sync::atomic::AtomicBool,
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
        let mut work_buckets = EnumMap::from_fn(|stage| {
            let active = stage == WorkBucketStage::Unconstrained;
            WorkBucket::new(active, worker_monitor.clone())
        });

        work_buckets[WorkBucketStage::Unconstrained].enable_prioritized_queue();

        // Set the open condition of each bucket.
        {
            let first_stw_stage = WorkBucketStage::first_stw_stage();
            let mut open_stages: Vec<WorkBucketStage> = vec![first_stw_stage];
            let stages = (0..WorkBucketStage::LENGTH).map(WorkBucketStage::from_usize);
            for stage in stages {
                {
                    if stage == WorkBucketStage::ConcurrentSentinel {
                        work_buckets[stage].set_open_condition(
                            move |scheduler: &GCWorkScheduler<VM>| {
                                scheduler.work_buckets[WorkBucketStage::Unconstrained].is_drained()
                            },
                        );
                        open_stages.push(stage);
                        continue;
                    }
                }
                // Unconstrained is always open.
                // The first STW stage (Prepare) will be opened when the world stopped
                // (i.e. when all mutators are suspended).
                if stage != WorkBucketStage::Unconstrained && stage != first_stw_stage {
                    // Other work packets will be opened after previous stages are done
                    // (i.e their buckets are drained and all workers parked).
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

        Arc::new(Self {
            work_buckets,
            worker_group,
            worker_monitor,
            affinity,
            postponed_concurrent_work: spin::RwLock::new(crossbeam::deque::Injector::new()),
            postponed_concurrent_work_prioritized: spin::RwLock::new(
                crossbeam::deque::Injector::new(),
            ),
            in_gc_pause: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn postpone(&self, w: impl GCWork<VM>) {
        self.postponed_concurrent_work.read().push(Box::new(w))
    }

    pub fn postpone_prioritized(&self, w: impl GCWork<VM>) {
        self.postponed_concurrent_work_prioritized
            .read()
            .push(Box::new(w))
    }

    pub fn postpone_dyn(&self, w: Box<dyn GCWork<VM>>) {
        self.postponed_concurrent_work.read().push(w)
    }

    pub fn postpone_dyn_prioritized(&self, w: Box<dyn GCWork<VM>>) {
        self.postponed_concurrent_work_prioritized.read().push(w)
    }

    pub fn postpone_all(&self, ws: Vec<Box<dyn GCWork<VM>>>) {
        let postponed_concurrent_work = self.postponed_concurrent_work.read();
        ws.into_iter()
            .for_each(|w| postponed_concurrent_work.push(w));
    }

    pub fn postpone_all_prioritized(&self, ws: Vec<Box<dyn GCWork<VM>>>) {
        let postponed_concurrent_work = self.postponed_concurrent_work_prioritized.read();
        ws.into_iter()
            .for_each(|w| postponed_concurrent_work.push(w));
    }

    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().worker_count()
    }

    /// Create GC threads for the first time.  It will also create the `GCWorker` instances.
    ///
    /// Currently GC threads only include worker threads, and we currently have only one worker
    /// group.  We may add more worker groups in the future.
    pub fn spawn_gc_threads(self: &Arc<Self>, mmtk: &'static MMTK<VM>, tls: VMThread) {
        self.worker_group.initial_spawn(tls, mmtk);
    }

    /// Ask all GC workers to exit for forking.
    pub fn stop_gc_threads_for_forking(self: &Arc<Self>) {
        self.worker_group.prepare_surrender_buffer();

        debug!("A mutator is requesting GC threads to stop for forking...");
        self.worker_monitor.make_request(WorkerGoal::StopForFork);
    }

    /// Surrender the `GCWorker` struct of a GC worker when it exits.
    pub fn surrender_gc_worker(&self, worker: Box<GCWorker<VM>>) {
        let all_surrendered = self.worker_group.surrender_gc_worker(worker);

        if all_surrendered {
            debug!(
                "All {} workers surrendered.",
                self.worker_group.worker_count()
            );
            self.worker_monitor.on_all_workers_exited();
        }
    }

    /// Respawn GC threads after forking.  This will reuse the `GCWorker` instances of stopped
    /// workers.  `tls` is the VM thread that requests GC threads to be re-spawn, and will be
    /// passed down to [`crate::vm::Collection::spawn_gc_thread`].
    pub fn respawn_gc_threads_after_forking(self: &Arc<Self>, tls: VMThread) {
        self.worker_group.respawn(tls)
    }

    /// Resolve the affinity of a thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        self.affinity.resolve_affinity(thread);
    }

    /// Request a GC to be scheduled.  Called by mutator via `GCRequester`.
    pub(crate) fn request_schedule_collection(&self) {
        debug!("A mutator is sending GC-scheduling request to workers...");
        self.worker_monitor.make_request(WorkerGoal::Gc);
    }

    /// Add the `ScheduleCollection` packet.  Called by the last parked worker.
    fn add_schedule_collection_packet(&self) {
        // We are still holding the mutex `WorkerMonitor::sync`.  Do not notify now.
        self.work_buckets[WorkBucketStage::Unconstrained].add_no_notify(ScheduleCollection);
    }

    /// Schedule all the common work packets
    pub fn schedule_common_work<C: GCWorkContext<VM = VM>>(&self, plan: &'static C::PlanType) {
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
                .add(SoftRefProcessing::<C::DefaultProcessEdges>::new());
            self.work_buckets[WorkBucketStage::WeakRefClosure].add(WeakRefProcessing::<VM>::new());
            self.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());

            use crate::util::reference_processor::RefForwarding;
            if plan.constraints().needs_forward_after_liveness {
                self.work_buckets[WorkBucketStage::RefForwarding]
                    .add(RefForwarding::<C::DefaultProcessEdges>::new());
            }

            use crate::util::reference_processor::RefEnqueue;
            self.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }

        // Finalization
        if !*plan.base().options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            self.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<C::DefaultProcessEdges>::new());
            // forward refs
            if plan.constraints().needs_forward_after_liveness {
                self.work_buckets[WorkBucketStage::FinalizableForwarding]
                    .add(ForwardFinalization::<C::DefaultProcessEdges>::new());
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
            .set_sentinel(Box::new(VMProcessWeakRefs::<C::DefaultProcessEdges>::new()));

        if plan.constraints().needs_forward_after_liveness {
            // VM-specific weak ref forwarding
            self.work_buckets[WorkBucketStage::VMRefForwarding]
                .add(VMForwardWeakRefs::<C::DefaultProcessEdges>::new());
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
                bkt.set_as_enabled();
            }
        });
    }

    pub fn reset_state(&self) {
        let first_stw_stage = WorkBucketStage::first_stw_stage();
        self.work_buckets.iter().for_each(|(id, bkt)| {
            if id != WorkBucketStage::Unconstrained && id != first_stw_stage {
                bkt.deactivate();
                bkt.set_as_enabled();
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

    pub(super) fn set_in_gc_pause(&self, in_gc_pause: bool) {
        self.in_gc_pause
            .store(in_gc_pause, std::sync::atomic::Ordering::SeqCst);
        for wb in self.work_buckets.values() {
            wb.set_in_concurrent(!in_gc_pause);
        }
    }

    pub fn in_concurrent(&self) -> bool {
        !self.in_gc_pause.load(std::sync::atomic::Ordering::SeqCst)
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
    pub(crate) fn poll(&self, worker: &GCWorker<VM>) -> PollResult<VM> {
        if let Some(work) = self.poll_schedulable_work(worker) {
            return Ok(work);
        }
        self.poll_slow(worker)
    }

    fn poll_slow(&self, worker: &GCWorker<VM>) -> PollResult<VM> {
        loop {
            // Retry polling
            if let Some(work) = self.poll_schedulable_work(worker) {
                return Ok(work);
            }

            let ordinal = worker.ordinal;
            self.worker_monitor
                .park_and_wait(ordinal, |goals| self.on_last_parked(worker, goals))?;
        }
    }

    /// Called when the last worker parked.  `goal` allows this function to inspect and change the
    /// current goal.
    fn on_last_parked(&self, worker: &GCWorker<VM>, goals: &mut WorkerGoals) -> LastParkedResult {
        let Some(ref current_goal) = goals.current() else {
            // There is no goal.  Find a request to respond to.
            return self.respond_to_requests(worker, goals);
        };

        match current_goal {
            WorkerGoal::Gc => {
                // We are in the progress of GC.

                // In stop-the-world GC, mutators cannot request for GC while GC is in progress.
                // When we support concurrent GC, we should remove this assertion.
                assert!(
                    !goals.debug_is_requested(WorkerGoal::Gc),
                    "GC request sent to WorkerMonitor while GC is still in progress."
                );

                // We are in the middle of GC, and the last GC worker parked.
                trace!("The last worker parked during GC.  Try to find more work to do...");

                // During GC, if all workers parked, all open buckets must have been drained.
                self.assert_all_activated_buckets_are_empty();

                // Find more work for workers to do.
                let found_more_work = self.find_more_work_for_workers();

                if found_more_work {
                    LastParkedResult::WakeAll
                } else {
                    // GC finished.
                    self.on_gc_finished(worker);

                    // Clear the current goal
                    goals.on_current_goal_completed();
                    self.respond_to_requests(worker, goals)
                }
            }
            WorkerGoal::StopForFork => {
                panic!(
                    "Worker {} parked again when it is asked to exit.",
                    worker.ordinal
                )
            }
        }
    }

    /// Respond to a worker reqeust.
    fn respond_to_requests(
        &self,
        worker: &GCWorker<VM>,
        goals: &mut WorkerGoals,
    ) -> LastParkedResult {
        assert!(goals.current().is_none());

        let Some(goal) = goals.poll_next_goal() else {
            // No requests.  Park this worker, too.
            return LastParkedResult::ParkSelf;
        };

        match goal {
            WorkerGoal::Gc => {
                trace!("A mutator requested a GC to be scheduled.");

                // We set the eBPF trace point here so that bpftrace scripts can start recording
                // work packet events before the `ScheduleCollection` work packet starts.
                probe!(mmtk, gc_start);

                {
                    let mut gc_start_time = worker.mmtk.state.gc_start_time.borrow_mut();
                    assert!(gc_start_time.is_none(), "GC already started?");
                    *gc_start_time = Some(Instant::now());
                }

                self.add_schedule_collection_packet();
                LastParkedResult::WakeSelf
            }
            WorkerGoal::StopForFork => {
                trace!("A mutator wanted to fork.");
                LastParkedResult::WakeAll
            }
        }
    }

    /// Find more work for workers to do.  Return true if more work is available.
    fn find_more_work_for_workers(&self) -> bool {
        if self.worker_group.has_designated_work() {
            trace!("Some workers have designated work.");
            return true;
        }

        // See if any bucket has a sentinel.
        if self.schedule_sentinels() {
            trace!("Some sentinels are scheduled.");
            return true;
        }

        // Try to open new buckets.
        if self.update_buckets() {
            trace!("Some buckets are opened.");
            return true;
        }

        // If all of the above failed, it means GC has finished.
        false
    }

    /// Called when GC has finished, i.e. when all work packets have been executed.
    fn on_gc_finished(&self, worker: &GCWorker<VM>) {
        // All GC workers must have parked by now.
        debug_assert!(!self.worker_group.has_designated_work());
        debug_assert!(self.all_buckets_empty());

        // Deactivate all work buckets to prepare for the next GC.
        self.deactivate_all();
        self.debug_assert_all_buckets_deactivated();

        let mmtk = worker.mmtk;

        let (queue, pqueue) = self.schedule_postponed_concurrent_packets();

        // Tell GC trigger that GC ended - this happens before we resume mutators.
        mmtk.gc_trigger.policy.on_gc_end(mmtk);

        // All other workers are parked, so it is safe to access the Plan instance mutably.
        probe!(mmtk, plan_end_of_gc_begin);
        let plan_mut: &mut dyn Plan<VM = VM> = unsafe { mmtk.get_plan_mut() };
        plan_mut.end_of_gc(worker.tls);
        probe!(mmtk, plan_end_of_gc_end);

        // Compute the elapsed time of the GC.
        let start_time = {
            let mut gc_start_time = worker.mmtk.state.gc_start_time.borrow_mut();
            gc_start_time.take().expect("GC not started yet?")
        };
        let elapsed = start_time.elapsed();

        info!(
            "End of GC ({}/{} pages, took {} ms)",
            mmtk.get_plan().get_reserved_pages(),
            mmtk.get_plan().get_total_pages(),
            elapsed.as_millis()
        );

        // USDT tracepoint for the end of GC.
        probe!(mmtk, gc_end);

        if *mmtk.get_options().count_live_bytes_in_gc {
            // Aggregate the live bytes
            let live_bytes = mmtk
                .scheduler
                .worker_group
                .get_and_clear_worker_live_bytes();
            let mut live_bytes_in_last_gc = mmtk.state.live_bytes_in_last_gc.borrow_mut();
            *live_bytes_in_last_gc = mmtk.aggregate_live_bytes_in_last_gc(live_bytes);
            // Logging
            for (space_name, &stats) in live_bytes_in_last_gc.iter() {
                info!(
                    "{} = {} pages ({:.1}% live)",
                    space_name,
                    stats.used_pages,
                    stats.live_bytes as f64 * 100.0 / stats.used_bytes as f64,
                );
            }
        }

        #[cfg(feature = "extreme_assertions")]
        if crate::util::slot_logger::should_check_duplicate_slots(mmtk.get_plan()) {
            // reset the logging info at the end of each GC
            mmtk.slot_logger.reset();
        }
        mmtk.get_plan().gc_pause_end();
        // Reset the triggering information.
        mmtk.state.reset_collection_trigger();

        // Set to NotInGC after everything, and right before resuming mutators.
        mmtk.set_gc_status(GcStatus::NotInGC);
        <VM as VMBinding>::VMCollection::resume_mutators(worker.tls);

        self.set_in_gc_pause(false);
        self.schedule_concurrent_packets(queue, pqueue);
        self.debug_assert_all_buckets_deactivated();
    }

    pub fn enable_stat(&self) {
        for worker in &self.worker_group.workers_shared {
            let worker_stat = worker.borrow_stat();
            worker_stat.enable();
        }
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.worker_group.workers_shared {
            let worker_stat = worker.borrow_stat();
            summary.merge(&worker_stat);
        }
        summary.harness_stat()
    }

    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.gc_requester.clear_request();
        let first_stw_bucket = &self.work_buckets[WorkBucketStage::first_stw_stage()];
        debug_assert!(!first_stw_bucket.is_activated());
        // Note: This is the only place where a bucket is opened without having all workers parked.
        // We usually require all workers to park before opening new buckets because otherwise
        // packets will be executed out of order.  However, since `Prepare` is the first STW
        // bucket, and all subsequent buckets require all workers to park before opening, workers
        // cannot execute work packets out of order.  This is not generally true if we are not
        // opening the first STW bucket.  In the future, we should redesign the opening condition
        // of work buckets to make the synchronization more robust,
        first_stw_bucket.activate();
        self.worker_monitor.notify_work_available(true);
    }

    fn schedule_postponed_concurrent_packets(&self) -> (PostponeQueue<VM>, PostponeQueue<VM>) {
        let mut queue = Injector::new();

        std::mem::swap::<PostponeQueue<VM>>(
            &mut queue,
            &mut self.postponed_concurrent_work.write(),
        );

        let mut pqueue = Injector::new();
        std::mem::swap::<PostponeQueue<VM>>(
            &mut pqueue,
            &mut self.postponed_concurrent_work_prioritized.write(),
        );
        (queue, pqueue)
    }

    pub(super) fn schedule_concurrent_packets(
        &self,
        queue: PostponeQueue<VM>,
        pqueue: PostponeQueue<VM>,
    ) {
        // crate::MOVE_CONCURRENT_MARKING_TO_STW.store(false, Ordering::SeqCst);
        // crate::PAUSE_CONCURRENT_MARKING.store(false, Ordering::SeqCst);
        let mut notify = false;
        if !queue.is_empty() {
            let old_queue = self.work_buckets[WorkBucketStage::Unconstrained].swap_queue(queue);
            debug_assert!(old_queue.is_empty());
            notify = true;
        }
        if !pqueue.is_empty() {
            let old_queue =
                self.work_buckets[WorkBucketStage::Unconstrained].swap_queue_prioritized(pqueue);
            debug_assert!(old_queue.is_empty());
            notify = true;
        }
        if notify {
            self.wakeup_all_concurrent_workers();
        }
    }

    pub fn wakeup_all_concurrent_workers(&self) {
        self.worker_monitor.notify_work_available(true);
    }
}
