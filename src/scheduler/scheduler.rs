use super::gc_work::ScheduleCollection;
use super::stat::SchedulerStat;
use super::work_bucket::*;
use super::worker::{GCWorker, ThreadId, WorkerGroup, WorkerMonitor};
use super::*;
use crate::global_state::GcStatus;
use crate::mmtk::MMTK;
use crate::scheduler::worker::LastParkedResult;
use crate::util::opaque_pointer::*;
use crate::util::options::AffinityKind;
use crate::util::rust_util::array_from_fn;
use crate::vm::Collection;
use crate::vm::VMBinding;
use crate::Plan;
use crossbeam::deque::Steal;
use enum_map::{Enum, EnumMap};
use std::collections::HashMap;
use std::sync::Arc;

pub struct GCWorkScheduler<VM: VMBinding> {
    /// Work buckets
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<VM>>,
    /// Workers
    pub(crate) worker_group: Arc<WorkerGroup<VM>>,
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

        Arc::new(Self {
            work_buckets,
            worker_group,
            worker_monitor,
            affinity,
        })
    }

    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().worker_count()
    }

    /// Create GC threads, including all workers.
    pub fn spawn_gc_threads(self: &Arc<Self>, mmtk: &'static MMTK<VM>, tls: VMThread) {
        self.worker_group.spawn(mmtk, tls)
    }

    /// Resolve the affinity of a thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        self.affinity.resolve_affinity(thread);
    }

    /// Schedule collection.  Called via `GCRequester`.
    /// Because this function is called by a mutator thread, we only add a `ScheduleCollection`
    /// work packet here so that a GC worker can wake up later and actually schedule the work for a
    /// collection.
    pub(crate) fn mutator_schedule_collection(&self) {
        debug!("Adding ScheduleCollection work packet upon mutator request.");
        // Add a ScheduleCollection work packet.  It is the seed of other work packets.
        self.work_buckets[WorkBucketStage::Unconstrained].add(ScheduleCollection);
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
    pub(crate) fn assert_all_activated_buckets_are_empty(&self, worker: &GCWorker<VM>) {
        let mut error_example = None;
        for (id, bucket) in self.work_buckets.iter() {
            if bucket.is_activated() && !bucket.is_empty() {
                error!("Work bucket {:?} is active but not empty!", id);
                // This error can be hard to reproduce.
                // If an error happens in the release build where logs are turned off,
                // we should show at least one abnormal bucket in the panic message
                // so that we still have some information for debugging.
                error_example = Some(id);

                while !bucket.is_empty() {
                    match bucket.poll(&worker.local_work_buffer) {
                        Steal::Success(w) => {
                            error!("  Bucket {:?} has {:?}", id, w.get_type_name());
                        },
                        Steal::Retry => continue,
                        _ => {}
                    }
                }
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

            self.worker_monitor.park_and_wait(worker, || {
                // This is the last worker parked.

                // Test whether we are doing GC.
                if worker.mmtk.gc_in_progress() {
                    // We are in the middle of GC, and the last GC worker parked.
                    trace!("GC is scheduled.  Try to find more work to do...");

                    // During GC, if all workers parked, all open buckets must have been drained.
                    self.assert_all_activated_buckets_are_empty(worker);

                    // Find more work for workers to do.
                    let found_more_work = self.find_more_work_for_workers();

                    if found_more_work {
                        LastParkedResult::WakeAll
                    } else {
                        // GC finished.
                        let scheduled_next_gc = self.on_gc_finished(worker);
                        if scheduled_next_gc {
                            LastParkedResult::WakeSelf
                        } else {
                            LastParkedResult::ParkSelf
                        }
                    }
                } else {
                    trace!("GC is not scheduled.  Wait for the first GC.");
                    // GC is not scheduled.  Do nothing.
                    // Note that when GC worker threads has just been created, they will try to get
                    // work packets to execute.  But since the first GC has not started, yet, there
                    // is not any work packets to execute, yet.  Therefore, all workers will park,
                    // and the last parked worker will reach here.  In that case, we should simply
                    // let workers wait until the first GC starts, instead of trying to open more
                    // buckets.
                    // If a GC worker spuriously wakes up when GC is not scheduled, it should not
                    // do anything, either.
                    LastParkedResult::ParkSelf
                }
            });
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
    /// Return `true` if it scheduled the next GC immediately.
    fn on_gc_finished(&self, worker: &GCWorker<VM>) -> bool {
        // All GC workers (except this one) must have parked by now.
        debug_assert!(!self.worker_group.has_designated_work());
        debug_assert!(self.all_buckets_empty());

        // Deactivate all work buckets to prepare for the next GC.
        self.deactivate_all();
        self.debug_assert_all_buckets_deactivated();

        let mmtk = worker.mmtk;

        // Tell GC trigger that GC ended - this happens before we resume mutators.
        mmtk.gc_trigger.policy.on_gc_end(mmtk);

        // Compute the elapsed time of the GC.
        let gc_start = {
            let mut guard = mmtk.state.gc_start_time.borrow_mut();
            guard.take().expect("gc_start_time was not set")
        };
        let elapsed = gc_start.elapsed();

        info!(
            "End of GC ({}/{} pages, took {} ms)",
            mmtk.get_plan().get_reserved_pages(),
            mmtk.get_plan().get_total_pages(),
            elapsed.as_millis()
        );

        #[cfg(feature = "count_live_bytes_in_gc")]
        {
            let live_bytes = mmtk.state.get_live_bytes_in_last_gc();
            let used_bytes =
                mmtk.get_plan().get_used_pages() << crate::util::constants::LOG_BYTES_IN_PAGE;
            debug_assert!(
                live_bytes <= used_bytes,
                "Live bytes of all live objects ({} bytes) is larger than used pages ({} bytes), something is wrong.",
                live_bytes, used_bytes
            );
            info!(
                "Live objects = {} bytes ({:04.1}% of {} used pages)",
                live_bytes,
                live_bytes as f64 * 100.0 / used_bytes as f64,
                mmtk.get_plan().get_used_pages()
            );
        }

        // All other workers are parked, so it is safe to access the Plan instance mutably.
        let plan_mut: &mut dyn Plan<VM = VM> = unsafe { mmtk.get_plan_mut() };
        plan_mut.end_of_gc(worker.tls);

        #[cfg(feature = "extreme_assertions")]
        if crate::util::edge_logger::should_check_duplicate_edges(mmtk.get_plan()) {
            // reset the logging info at the end of each GC
            mmtk.edge_logger.reset();
        }

        // Reset the triggering information.
        mmtk.state.reset_collection_trigger();

        // Set to NotInGC after everything, and right before resuming mutators.
        mmtk.set_gc_status(GcStatus::NotInGC);
        <VM as VMBinding>::VMCollection::resume_mutators(worker.tls);

        // GC offically ends here.
        probe!(mmtk, gc_end);

        // Notify the `GCRequester` that GC has finished.
        let should_schedule_gc_now = mmtk.gc_requester.on_gc_finished();
        if should_schedule_gc_now {
            // We should schedule the next GC immediately.  This means GC was triggered between
            // `clear_request` (when stacks were scanned) and `on_gc_finished` (right above).  This
            // can happen if
            // 1.  It is concurrent GC, and a mutator triggered another GC while the current GC was
            //     still running, or
            // 2.  It is STW GC, but after the invocation of `resume_mutators` above, one mutator
            //     ran so fast that it triggered a GC before we called `on_gc_finished`.
            // Note that we are holding the `WorkerMonitor` mutex, and cannot notify workers now.
            // When this function returns, the current worker should continue to execute the newly
            // added `ScheduleCollection` work packet.
            debug!("GC already requested before `on_gc_finished`.  Add ScheduleCollection now.");
            self.work_buckets[WorkBucketStage::Unconstrained].add_no_notify(ScheduleCollection);
            true
        } else {
            false
        }
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
}
