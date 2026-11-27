use super::gc_work::*;
use super::scheduler::{AlarmThread, IncrementalMarkSweepTimer};
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::global::{BasePlan, CommonPlan, CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::{AllocationSemantics, Plan, PlanConstraints};
use crate::policy::marksweepspace::native_ms::{MarkSweepSpace, MAX_OBJECT_SIZE};
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::deadline::{now_nanos, DeadlineClock};
use crate::util::gray_pool::GrayPool;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::log_bit::UnlogBitsOperation;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::VMWorkerThread;
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;
use atomic::Atomic;
use enum_map::EnumMap;
use mmtk_macros::{HasSpaces, PlanTraceObject};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Current state of incremental marking cycle.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::NoUninit)]
pub enum IncrementalMarkSweepState {
    Idle = 0,
    /// Marking cycle triggered, the alarm thread starts the cycle.
    Triggered = 1,
    Marking = 2,
    /// Marking finished at the end of an increment.
    ReadyToFinish = 3,
    /// The cycle's `FinalMark` is done. Increments sweep the blocks it marked.
    Sweeping = 4,
}

#[derive(HasSpaces, PlanTraceObject)]
pub struct IncrementalMarkSweep<VM: VMBinding> {
    #[space]
    pub ms: MarkSweepSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,

    pub(super) state: Atomic<IncrementalMarkSweepState>,
    current_pause: Atomic<Option<Pause>>,
    previous_pause: Atomic<Option<Pause>>,
    /// Finish the current cycle in the next pause, without a deadline.
    sync_requested: AtomicBool,
    /// The current sweep increment runs without a deadline, as an allocation failed.
    finish_sweep: AtomicBool,
    /// Incremental mode enabled (false = every GC is `Full`).
    incremental: bool,

    /// Marked objects that still need scanning. Objects end up here
    /// only if `IncrementalMarkSweepMark` hits the deadline, and the next increment scans them.
    pub(super) gray: GrayPool,
    /// SATB-logged objects (not yet marked) flushed by mutator barriers.
    pub(super) satb: GrayPool,

    pub(super) deadline: DeadlineClock,
    /// Set when the deadline expires or marking drains. Cleared at the next pause start.
    yielding: AtomicBool,
    /// The current increment ends: marking packets spill instead of scanning.
    ending: AtomicBool,
    yield_lock: Mutex<()>,
    pub(super) timer: IncrementalMarkSweepTimer,
    alarm: AlarmThread,

    /// Start a cycle when used bytes exceed this.
    trigger_bytes: AtomicUsize,
    initial_trigger_percent: usize,
    headroom_bytes: usize,
}

pub const INCREMENTAL_MARKSWEEP_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    max_non_los_default_alloc_bytes: MAX_OBJECT_SIZE,
    may_trace_duplicate_edges: true,
    needs_prepare_mutator: true,
    barrier: crate::BarrierSelector::SATBBarrier,
    needs_log_bit: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for IncrementalMarkSweep<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &INCREMENTAL_MARKSWEEP_CONSTRAINTS
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        let heap_full = self.base().collection_required(self, space_full);
        match self.state() {
            IncrementalMarkSweepState::Idle | IncrementalMarkSweepState::Triggered => {
                if heap_full {
                    // The heap is full: collect now.
                    return true;
                }
                // Only mark a cycle as triggered. The alarm thread starts it.
                let used = self.get_used_pages() << LOG_BYTES_IN_PAGE;
                if self.incremental && used > self.trigger_bytes() {
                    let _ = self.state.compare_exchange(
                        IncrementalMarkSweepState::Idle,
                        IncrementalMarkSweepState::Triggered,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                }
                false
            }
            // Sweeping releases no pages, so a full heap needs a collection. The next cycle
            // can only be triggered once sweeping is done.
            IncrementalMarkSweepState::Sweeping => heap_full,
            IncrementalMarkSweepState::Marking | IncrementalMarkSweepState::ReadyToFinish => {
                if heap_full {
                    // Finish the cycle stop-the-world, or run one quantum now.
                    if self.synchronous_gc_on_oom() {
                        self.sync_requested.store(true, Ordering::SeqCst);
                    }
                    return true;
                }
                // Otherwise the alarm thread decides
                false
            }
        }
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let gs = &self.base().global_state;
        let user = gs.is_user_triggered_collection();
        let attempts = gs.cur_collection_attempts.load(Ordering::SeqCst);
        let heap_full = self.base().gc_trigger.is_heap_full();
        let pause = match self.state() {
            IncrementalMarkSweepState::Idle | IncrementalMarkSweepState::Triggered => {
                if !self.incremental
                    || user
                    || attempts > 1
                    || (heap_full && self.synchronous_gc_on_oom())
                {
                    Pause::Full
                } else {
                    Pause::InitialMark
                }
            }
            IncrementalMarkSweepState::Marking => {
                if self.sync_requested.load(Ordering::SeqCst) || user || attempts > 1 {
                    Pause::FinalMark
                } else {
                    Pause::Increment
                }
            }
            IncrementalMarkSweepState::ReadyToFinish => Pause::FinalMark,
            IncrementalMarkSweepState::Sweeping => {
                if user || attempts > 1 || (heap_full && self.synchronous_gc_on_oom()) {
                    // A `Full` sweeps everything, including what is left to sweep.
                    Pause::Full
                } else {
                    self.finish_sweep.store(heap_full, Ordering::SeqCst);
                    Pause::Increment
                }
            }
        };
        self.current_pause.store(Some(pause), Ordering::SeqCst);
        debug!("IncrementalMarkSweep: {:?}", pause);
        self.timer.on_stop_mutators();

        let refs_enabled = matches!(pause, Pause::Full | Pause::FinalMark);
        self.set_ref_closure_buckets_enabled(refs_enabled);

        match pause {
            Pause::Full => {
                scheduler.schedule_common_work::<IncrementalMarkSweepSTWContext<VM>>(self)
            }
            Pause::InitialMark => {
                use crate::scheduler::gc_work::{Prepare, StopMutators};
                scheduler.work_buckets[WorkBucketStage::Unconstrained]
                    .add(StopMutators::<IncrementalMarkSweepIncContext<VM>>::new());
                scheduler.work_buckets[WorkBucketStage::Prepare]
                    .add(Prepare::<IncrementalMarkSweepIncContext<VM>>::new(self));
                // Roots are turned into IncrementalMarkSweepMark packets by the roots factory.
                scheduler.work_buckets[WorkBucketStage::Closure]
                    .set_sentinel(Box::new(IncrementalMarkSweepEndOfMarking::<VM>::new()));
            }
            Pause::Increment if self.state() == IncrementalMarkSweepState::Sweeping => {
                use crate::scheduler::gc_work::StopMutators;
                scheduler.work_buckets[WorkBucketStage::Unconstrained]
                    .add(StopMutators::<IncrementalMarkSweepIncContext<VM>>::new_no_scan_roots());
                for _ in 0..scheduler.num_workers() {
                    scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork]
                        .add(IncrementalMarkSweepSweep::<VM>::new());
                }
            }
            Pause::Increment => {
                use crate::scheduler::gc_work::StopMutators;
                scheduler.work_buckets[WorkBucketStage::Unconstrained]
                    .add(StopMutators::<IncrementalMarkSweepIncContext<VM>>::new_no_scan_roots());
                scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork].add(
                    IncrementalMarkSweepResumeMarking::<VM>::new(
                        WorkBucketStage::FinishConcurrentWork,
                    ),
                );
                scheduler.work_buckets[WorkBucketStage::Closure]
                    .set_sentinel(Box::new(IncrementalMarkSweepEndOfMarking::<VM>::new()));
            }
            Pause::FinalMark => {
                use crate::scheduler::gc_work::StopMutators;
                scheduler.work_buckets[WorkBucketStage::Unconstrained]
                    .add(StopMutators::<IncrementalMarkSweepIncContext<VM>>::new_no_scan_roots());
                scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork].add(
                    IncrementalMarkSweepResumeMarking::<VM>::new(
                        WorkBucketStage::FinishConcurrentWork,
                    ),
                );
                self.schedule_final_mark_tail(scheduler);
            }
            Pause::RefCount => unreachable!(),
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &super::mutator::ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::Full => {
                self.common.prepare(tls, true);
                self.ms.prepare(true);
            }
            Pause::InitialMark => {
                self.common.prepare(tls, true);
                self.ms.prepare(true);
                // SATB: every existing object must be unlogged so the first write logs it.
                self.ms.set_side_log_bits();
                self.common
                    .schedule_unlog_bits_op(UnlogBitsOperation::BulkSet);
            }
            _ => unreachable!("prepare is only scheduled for Full and InitialMark"),
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::Full => {
                self.ms.set_defer_sweep(false);
                self.ms.release();
                self.common.release(tls, true);
            }
            Pause::FinalMark => {
                debug_assert!(self.gray.is_empty() && self.satb.is_empty());
                // Sweep in later increments, unless someone is waiting for this collection.
                let gs = &self.base().global_state;
                let urgent = self.sync_requested.load(Ordering::SeqCst)
                    || gs.is_user_triggered_collection()
                    || gs.cur_collection_attempts.load(Ordering::SeqCst) > 1;
                self.ms.set_defer_sweep(!urgent);
                self.ms.release();
                self.common.release(tls, true);
                self.ms.clear_side_log_bits();
                self.common
                    .schedule_unlog_bits_op(UnlogBitsOperation::BulkClear);
            }
            _ => unreachable!("release is only scheduled for Full and FinalMark"),
        }
    }

    fn on_pause_start(&self, mmtk: &'static MMTK<VM>) {
        let pause = self.current_pause().unwrap();
        let slice = self.timer.start_gc_increment();
        self.yielding.store(false, Ordering::SeqCst);
        self.ending.store(false, Ordering::SeqCst);
        match pause {
            Pause::Increment
                if self.state() == IncrementalMarkSweepState::Sweeping
                    && self.finish_sweep.load(Ordering::SeqCst) =>
            {
                self.deadline.disarm()
            }
            Pause::InitialMark | Pause::Increment => self.deadline.arm(slice),
            Pause::FinalMark | Pause::Full => self.begin_final_mark(),
            Pause::RefCount => unreachable!(),
        }
        if matches!(pause, Pause::Full | Pause::InitialMark) {
            mmtk.gc_trigger.policy.on_gc_start(mmtk);
        }
    }

    fn on_pause_end(&mut self, mmtk: &'static MMTK<VM>, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        #[cfg(all(test, feature = "mock_test"))]
        let sync = self.sync_requested.load(Ordering::SeqCst);
        match pause {
            Pause::InitialMark | Pause::Increment => {
                if pause == Pause::InitialMark {
                    self.set_allocate_black(true);
                }

                // MMTk counts a pause as a failed allocation unless a mutator has allocated
                // through the slow path since the previous pause. Fast-path allocations and
                // alarm-triggered increments do not count, so set the flag to keep the second
                // increment from becoming a `FinalMark`.
                //
                // TODO: better mechanism for this?
                mmtk.state.allocation_success.store(true, Ordering::SeqCst);
                let next = if self.state() == IncrementalMarkSweepState::Sweeping {
                    self.state_after_sweeping()
                } else if self.gray.is_empty() && self.satb.is_empty() {
                    IncrementalMarkSweepState::ReadyToFinish
                } else {
                    IncrementalMarkSweepState::Marking
                };
                self.state.store(next, Ordering::SeqCst);
            }
            Pause::FinalMark | Pause::Full => {
                self.ms.end_of_gc();
                self.common.on_pause_end(tls);
                self.state
                    .store(self.state_after_sweeping(), Ordering::SeqCst);
                self.sync_requested.store(false, Ordering::SeqCst);
                self.rearm_trigger();
                mmtk.gc_trigger.policy.on_gc_end(mmtk);
            }
            Pause::RefCount => unreachable!(),
        }
        self.deadline.disarm();
        self.timer.on_pause_end();
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
    }

    fn on_collection_initialized(&'static self, mmtk: &'static MMTK<VM>) {
        if !self.incremental {
            return;
        }
        self.alarm.running.store(true, Ordering::SeqCst);
        let plan: &'static Self = self;
        let handle = std::thread::Builder::new()
            .name("ims-alarm".into())
            .spawn(move || plan.alarm_loop(mmtk))
            .expect("failed to spawn the IncrementalMarkSweep alarm thread");
        *self.alarm.handle.lock().unwrap() = Some(handle);
    }

    fn on_shutdown(&self) {
        self.alarm.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.alarm.handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    fn current_gc_may_move_object(&self) -> bool {
        false
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        // A `FinalMark` keeps the objects allocated black during the cycle.
        !matches!(
            self.previous_pause.load(Ordering::SeqCst),
            Some(Pause::InitialMark | Pause::Increment | Pause::FinalMark)
        )
    }

    fn get_used_pages(&self) -> usize {
        self.ms.reserved_pages() + self.common.get_used_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }
    fn base_mut(&mut self) -> &mut BasePlan<VM> {
        &mut self.common.base
    }
    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
    fn concurrent(&self) -> Option<&dyn ConcurrentPlan<VM = VM>> {
        Some(self)
    }
}

impl<VM: VMBinding> ConcurrentPlan for IncrementalMarkSweep<VM> {
    fn concurrent_work_in_progress(&self) -> bool {
        self.is_marking()
    }
    fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }
    fn on_concurrent_work_interrupted(&self) {}
    fn on_concurrent_work_drained(&self) {}
}

impl<VM: VMBinding> IncrementalMarkSweep<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        if !cfg!(feature = "eager_sweeping") {
            panic!(
                "IncrementalMarkSweep requires the `eager_sweeping` Cargo feature: lazy sweeping would sweep \
                 blocks with partial mark bits while an incremental cycle is in progress"
            );
        }

        let log_bits_on_side = VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.is_on_side();
        if !log_bits_on_side {
            warn!(
                "IncrementalMarkSweep bulk-sets log bits and needs them on the side; \
                 falling back to stop-the-world mode"
            );
        }
        let options = args.options.clone();
        let mut global_specs = crate::util::metadata::extract_side_metadata(&[
            *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
        ]);
        MarkSweepSpace::<VM>::extend_global_side_metadata_specs(&mut global_specs);
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &INCREMENTAL_MARKSWEEP_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&global_specs),
        };
        let ms = MarkSweepSpace::new(plan_args.get_normal_space_args(
            "incremental_marksweep_ms",
            true,
            false,
            VMRequest::discontiguous(),
        ));
        let common = CommonPlan::new(plan_args);

        let beat = Duration::from_micros(*options.incremental_marksweep_beat_us as u64);
        let window = if *options.incremental_marksweep_window_us == 0 {
            beat * 20
        } else {
            Duration::from_micros(*options.incremental_marksweep_window_us as u64)
        };
        let target = *options.incremental_marksweep_target_utilization as f64 / 100.0;

        let incremental = log_bits_on_side;

        IncrementalMarkSweep {
            ms,
            common,
            state: Atomic::new(IncrementalMarkSweepState::Idle),
            current_pause: Atomic::new(None),
            previous_pause: Atomic::new(None),
            sync_requested: AtomicBool::new(false),
            finish_sweep: AtomicBool::new(false),
            incremental,
            gray: GrayPool::default(),
            satb: GrayPool::default(),
            deadline: DeadlineClock::default(),
            yielding: AtomicBool::new(false),
            ending: AtomicBool::new(false),
            yield_lock: Mutex::new(()),
            timer: IncrementalMarkSweepTimer::new(beat, window, target),
            alarm: AlarmThread::default(),
            trigger_bytes: AtomicUsize::new(0),
            initial_trigger_percent: *options.incremental_marksweep_trigger_percent,
            headroom_bytes: *options.incremental_marksweep_headroom_bytes,
        }
    }

    /// Whether the alarm thread has been started and not yet joined.
    #[cfg(all(test, feature = "mock_test"))]
    #[cfg_attr(not(feature = "eager_sweeping"), allow(dead_code))]
    pub(super) fn alarm_thread_running(&self) -> bool {
        self.alarm.handle.lock().unwrap().is_some()
    }

    pub(super) fn state(&self) -> IncrementalMarkSweepState {
        self.state.load(Ordering::SeqCst)
    }

    pub(super) fn is_marking(&self) -> bool {
        matches!(
            self.state(),
            IncrementalMarkSweepState::Marking | IncrementalMarkSweepState::ReadyToFinish
        )
    }

    /// The state after a pause that has swept, or left blocks to sweep.
    fn state_after_sweeping(&self) -> IncrementalMarkSweepState {
        if self.ms.has_unswept_blocks() {
            IncrementalMarkSweepState::Sweeping
        } else {
            IncrementalMarkSweepState::Idle
        }
    }

    fn synchronous_gc_on_oom(&self) -> bool {
        *self
            .base()
            .options
            .incremental_marksweep_synchronous_gc_on_oom
    }

    fn initial_trigger_bytes(&self) -> usize {
        (self.get_total_pages() << LOG_BYTES_IN_PAGE) / 100 * self.initial_trigger_percent
    }

    fn trigger_bytes(&self) -> usize {
        usize::max(
            self.trigger_bytes.load(Ordering::Relaxed),
            self.initial_trigger_bytes(),
        )
    }

    fn rearm_trigger(&self) {
        let in_use = self.get_used_pages() << LOG_BYTES_IN_PAGE;
        self.trigger_bytes.store(
            usize::max(self.initial_trigger_bytes(), self.headroom_bytes + in_use),
            Ordering::SeqCst,
        );
    }

    fn set_allocate_black(&self, black: bool) {
        use crate::plan::global::HasSpaces;
        self.for_each_space(&mut |s: &dyn Space<VM>| s.set_allocate_as_live(black));
    }

    fn set_ref_closure_buckets_enabled(&self, enabled: bool) {
        let s = &self.common.base.scheduler;
        s.work_buckets[WorkBucketStage::VMRefClosure].set_enabled(enabled);
        s.work_buckets[WorkBucketStage::WeakRefClosure].set_enabled(enabled);
        s.work_buckets[WorkBucketStage::FinalRefClosure].set_enabled(enabled);
        s.work_buckets[WorkBucketStage::SoftRefClosure].set_enabled(enabled);
        s.work_buckets[WorkBucketStage::PhantomRefClosure].set_enabled(enabled);
    }

    /// Whether to yield from the current increment. Called by `IncrementalMarkSweepMark`.
    pub(super) fn should_yield(&self) -> bool {
        if !self.deadline.is_armed() {
            // No deadline: the GC must run to completion.
            return false;
        }
        if self.ending.load(Ordering::SeqCst) {
            return true;
        }

        if !self.yielding.load(Ordering::SeqCst) && !self.deadline.expired() {
            return false;
        }
        self.yield_from_gc()
    }

    /// Whether the current increment has decided to end.
    pub(super) fn increment_ending(&self) -> bool {
        self.ending.load(Ordering::SeqCst)
    }

    fn yield_from_gc(&self) -> bool {
        let _guard = self.yield_lock.lock().unwrap();
        if self.ending.load(Ordering::SeqCst) {
            return true;
        }
        if !self.yielding.load(Ordering::SeqCst) && !self.deadline.expired() {
            // Another worker took a double beat meanwhile.
            return false;
        }
        self.yielding.store(true, Ordering::SeqCst);
        if self.timer.should_gc_double_beat() {
            self.deadline.arm(self.timer.start_gc_double_beat());
            return false;
        }
        self.ending.store(true, Ordering::SeqCst);
        true
    }

    /// Called when the marking work of an InitialMark or Increment pause drains. If marking
    /// is complete and the GC gets another beat, turn this pause into a FinalMark.
    pub(super) fn on_marking_drained(&'static self, scheduler: &GCWorkScheduler<VM>) {
        if self.ending.load(Ordering::SeqCst) || !self.gray.is_empty() || !self.satb.is_empty() {
            return;
        }
        let _guard = self.yield_lock.lock().unwrap();
        self.yielding.store(true, Ordering::SeqCst);
        if !self.timer.should_gc_double_beat() {
            self.ending.store(true, Ordering::SeqCst);
            return;
        }
        self.timer.start_gc_double_beat();
        debug!("IncrementalMarkSweep: marking complete, finishing the cycle in this pause");
        self.current_pause
            .store(Some(Pause::FinalMark), Ordering::SeqCst);
        self.begin_final_mark();
        self.set_ref_closure_buckets_enabled(true);
        self.schedule_final_mark_tail(scheduler);
    }

    /// Marking is complete or must complete without a deadline: stop allocating black.
    fn begin_final_mark(&self) {
        self.deadline.disarm();
        self.set_allocate_black(false);
    }

    /// Reference processing and release of a `FinalMark`.
    fn schedule_final_mark_tail(&'static self, scheduler: &GCWorkScheduler<VM>) {
        use crate::plan::tracing::gc_work::weakref::VMProcessWeakRefs;
        use crate::scheduler::gc_work::Release;
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<IncrementalMarkSweepIncContext<VM>>::new(self));
        #[cfg(feature = "sanity")]
        {
            use crate::util::sanity::sanity_checker::ScheduleSanityGC;
            scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC::<Self>::new(self));
        }

        type RefTrace<VM> = IncrementalMarkSweepTrace<VM>;
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, RefEnqueue, SoftRefProcessing, WeakRefProcessing,
            };
            scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<RefTrace<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }
        if !*self.base().options.no_finalizer {
            use crate::util::finalizable_processor::Finalization;
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<RefTrace<VM>>::new());
        }
        scheduler.work_buckets[WorkBucketStage::VMRefClosure]
            .set_sentinel(Box::new(VMProcessWeakRefs::<RefTrace<VM>>::new()));
    }

    /// The alarm thread loop. It only requests pauses while GC workers do the rest.
    fn alarm_loop(&'static self, mmtk: &'static MMTK<VM>) {
        let period = self.timer.alarm_period();
        let beat_nanos = self.timer.beat.as_nanos() as u64;
        while self.alarm.running.load(Ordering::SeqCst) {
            std::thread::sleep(period);
            if self.state() == IncrementalMarkSweepState::Idle || self.current_pause().is_some() {
                continue;
            }

            let start = self.timer.mutator_slice_start.load(Ordering::SeqCst);

            if now_nanos().saturating_sub(start) < beat_nanos {
                continue;
            }

            if self.timer.should_mutator_double_beat() {
                // The mutator gets another beat.
                self.timer
                    .mutator_slice_start
                    .store(now_nanos(), Ordering::SeqCst);
                continue;
            }
            mmtk.gc_trigger.request_from_internal_thread();
        }
    }
}
