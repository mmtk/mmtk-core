use crate::plan::barriers::HybridBarrier;
use crate::plan::concurrent::barrier::HybridBarrierSemantics;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::span::assist::{AssistOutcome, Assists};
use crate::plan::concurrent::span::gc_work::{
    ConcurrentSpanGCWorkContext, ConcurrentSpanSTWGCWorkContext, FinalMarkCheck, RaggedFlush,
    ScanFinalizableContents,
};
use crate::plan::concurrent::span::mutator::ALLOCATOR_MAPPING;
use crate::plan::concurrent::span::pacing::Pacing;
use crate::plan::concurrent::span::stats::SpanStats;
use crate::plan::concurrent::Pause;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::tracing::gc_work::weakref::VMProcessWeakRefs;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::policy::space::Space;
use crate::policy::spanspace::SpanSpace;
use crate::policy::spanspace::MAX_OBJECT_SIZE;
use crate::scheduler::gc_work::{ConcurrentScanStacks, Prepare, Release, StopMutators};
use crate::scheduler::{GCWorkScheduler, WorkBucketStage};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::log_bit::UnlogBitsOperation;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::VMWorkerThread;
use crate::vm::{Collection, ObjectModel, Scanning, VMBinding};
use crate::MMTK;

use atomic::Atomic;
use atomic::Ordering;
use enum_map::EnumMap;
use mmtk_macros::{HasSpaces, PlanTraceObject};
use std::sync::atomic::{AtomicBool, AtomicUsize};

type HybridBarrierType<VM> =
    HybridBarrier<HybridBarrierSemantics<VM, ConcurrentSpan<VM>, DEFAULT_TRACE>>;

/// A port of Go's garbage collector: a concurrent, non-moving, non-generational mark-sweep
/// collector over a size-segregated span heap.  See `docs/design/go-gc-port.md`.
///
/// A cycle is an `InitialMark` pause, concurrent marking, and a `FinalMark` pause:
/// - `InitialMark` scans VM roots, turns on [`HybridBarrier`] and allocate-black, and, unless
///   the binding cannot do it, leaves the mutator stacks to be scanned concurrently.
/// - When the `Concurrent` bucket drains, [`RaggedFlush`] rounds flush every mutator's barrier
///   buffer until a round finds no work (Go's `gcMarkDone`).  Then `FinalMark` is requested.
/// - `FinalMark` does no tracing.  If the mutators' buffers hold work, marking restarts
///   concurrently.  Otherwise it processes references and releases the heap (Go's mark
///   termination).
///
/// The heap is `SpanSpace`.  Sweeping follows Go: `sweepgen += 2` at the end of marking, then
/// sweep packets plus lazy sweeping by allocators.
///
/// `Pause::Full` (stop-the-world) is only used for user-requested collections, and when
/// concurrent marking is disabled.
#[derive(HasSpaces, PlanTraceObject)]
pub struct ConcurrentSpan<VM: VMBinding> {
    #[space]
    pub(in crate::plan) span: SpanSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
    current_pause: Atomic<Option<Pause>>,
    previous_pause: Atomic<Option<Pause>>,
    /// Decides when a cycle starts.
    pacing: Pacing,
    /// Mark assists.
    assists: Assists,
    /// Statistics for the evaluation.
    stats: SpanStats,
    /// The number of GC cycles completed.
    completed_cycles: AtomicUsize,
    /// Assists are on, and the binding can block mutators.
    assists_enabled: bool,
    /// Concurrent marking is in progress: from the end of `InitialMark` until a `FinalMark`
    /// pause decides that marking is done.
    marking: AtomicBool,
    /// A ragged flush round found no work, so `FinalMark` can be requested.
    mark_done: AtomicBool,
    /// A pause interrupted concurrent marking (for example the heap is full), so `FinalMark`
    /// finishes the marking in the pause.
    marking_interrupted: AtomicBool,
    /// The current `FinalMark` pause found work and restarted concurrent marking.
    restarting: AtomicBool,
    /// This cycle keeps every softly reachable object, and the binding reports soft referents as
    /// strong slots (design §5.14).  Decided at `InitialMark`.
    soft_refs_strong: AtomicBool,
    /// Statistics for the current cycle.
    ragged_rounds: AtomicUsize,
    restarts: AtomicUsize,
    stalls: AtomicUsize,
    /// Mutator stacks are scanned concurrently, not in `InitialMark`.
    concurrent_stack_scan: bool,
    /// The plan uses the SATB barrier instead of `HybridBarrier`.
    pub(in crate::plan) satb: bool,
}

/// The plan constraints for `ConcurrentSpan`.
pub const CONCURRENT_SPAN_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    max_non_los_default_alloc_bytes: MAX_OBJECT_SIZE,
    may_trace_duplicate_edges: true,
    // Mutators flush their allocation caches in prepare and release.
    needs_prepare_mutator: true,
    barrier: crate::BarrierSelector::HybridBarrier,
    ..PlanConstraints::default()
};

/// The plan constraints for `ConcurrentSpan` with the SATB barrier instead of `HybridBarrier`
/// (option `concurrent_span_satb_barrier`), for comparison runs (design §5.6).
pub const CONCURRENT_SPAN_SATB_CONSTRAINTS: PlanConstraints = PlanConstraints {
    barrier: crate::BarrierSelector::SATBBarrier,
    needs_log_bit: true,
    ..CONCURRENT_SPAN_CONSTRAINTS
};

impl<VM: VMBinding> Plan for ConcurrentSpan<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        if self.satb {
            &CONCURRENT_SPAN_SATB_CONSTRAINTS
        } else {
            &CONCURRENT_SPAN_CONSTRAINTS
        }
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        if self.marking.load(Ordering::SeqCst) {
            // Marking is done once a ragged flush round finds no work, and every packet it
            // created has been traced.
            if self.mark_done.load(Ordering::SeqCst)
                && self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent].is_drained()
            {
                return true;
            }
            // The heap is full during marking.  With allocation stalls (the default), the mutator
            // waits for the cycle to end (`stall_instead_of_pause`).  Without them, this requests
            // `FinalMark`, which finishes the marking in the pause, as `ConcurrentImmix` does.
            return self.base().collection_required(self, space_full);
        }

        // The heap size from the GC trigger is the hard limit.  Otherwise the pacer decides (Go:
        // `gcTrigger.test`, `runtime/mgc.go:706-725`).
        let required = || {
            self.base().collection_required(self, space_full) || {
                self.report_memory_stats();
                self.pacing.report_heap_live(self.heap_live_bytes());
                self.pacing.should_start_cycle()
            }
        };
        // Go's `gcStart` (`runtime/mgc.go:767`) first sweeps until the trigger no longer holds:
        // `for trigger.test() && sweepone() != ^uintptr(0) {}`.  Here this matters most for the
        // hard limit, which counts the pages of unswept garbage.  Without it, a mutator that
        // takes pages right after a cycle (for example after an allocation stall) starts the
        // next cycle at once, and the pause sweeps the heap.
        while required() {
            if self.span.sweep_one() == 0 {
                return true;
            }
        }
        false
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let pause = if self.marking.load(Ordering::SeqCst) {
            Pause::FinalMark
        } else if !self.concurrent_marking_supported()
            // A user-requested collection is stop-the-world unless it runs as a concurrent cycle
            // that blocks only the caller (`concurrent_user_collection`).
            || (self.base().global_state.is_user_triggered_collection()
                && !(self.concurrent_user_collection()
                    && VM::VMCollection::supports_blocking_mutators()))
        {
            Pause::Full
        } else {
            Pause::InitialMark
        };
        self.current_pause.store(Some(pause), Ordering::SeqCst);
        probe!(mmtk, concurrent_pause_determined, pause as usize);

        match pause {
            Pause::Full => {
                self.set_ref_closure_buckets_enabled(true);
                scheduler.schedule_common_work::<ConcurrentSpanSTWGCWorkContext<VM>>(self);
            }
            Pause::InitialMark => self.schedule_initial_mark(scheduler),
            Pause::FinalMark => self.schedule_final_mark(scheduler),
            Pause::RefCount => unreachable!(),
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn on_pause_start(&self, mmtk: &'static MMTK<VM>) {
        let pause = self.current_pause().unwrap();
        // Every pause starts a new GC cycle, except `FinalMark`, which continues the cycle
        // started by the preceding `InitialMark` pause.
        if pause != Pause::FinalMark {
            debug_assert!(!self.marking.load(Ordering::SeqCst));
            // This pause may have interrupted background sweeping.  `prepare_initial_mark`
            // sweeps what is left in the pause (Go's sweep termination), so drop the packets.
            let leftover =
                mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent].drain_all_packets();
            if !leftover.is_empty() {
                debug!("Dropped {} background sweep packet(s)", leftover.len());
            }
            // If background sweeping finished without a pause, `Concurrent` is still enabled.
            // Close it, or the root packets of this pause would run before `Prepare`.  The
            // scheduler opens it after the pause if it holds work.
            mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
            mmtk.gc_trigger.policy.on_gc_start(mmtk);
            if log::log_enabled!(log::Level::Info) {
                info!(
                    "Heap at cycle start: {} of {} MMTk pages reserved, SpanSpace: {} KiB in spans ({} unswept spans), heap_live {} KiB",
                    self.get_reserved_pages(),
                    self.get_total_pages(),
                    self.span.pages_in_use() * (crate::policy::spanspace::classes::BYTES_IN_SPAN_PAGE >> 10),
                    self.span.count_unswept(),
                    self.heap_live_bytes() >> 10,
                );
            }
            self.pacing.report_heap_live(self.heap_live_bytes());
            self.pacing.start_cycle();
        } else {
            // The barrier buffers were flushed into `Concurrent` when the mutators stopped.  Keep
            // that bucket closed during the pause: `FinalMarkCheck` decides what to do with them.
            mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
        }
        self.stats.pause_started();
        self.pacing.pause_started();
        info!("{:?} start", pause);
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        match self.current_pause().unwrap() {
            Pause::Full | Pause::InitialMark => {
                self.common.prepare(tls, true);
                if self.satb && self.current_pause() == Some(Pause::InitialMark) {
                    // The SATB barrier logs every object that existed when marking started.
                    self.span.set_side_log_bits();
                    self.common
                        .schedule_unlog_bits_op(UnlogBitsOperation::BulkSet);
                }
                // Sweep termination: finish the previous cycle's sweeping, start a mark epoch.
                let residue = self.span.prepare_initial_mark();
                self.stats.sweep_residue(residue);
            }
            Pause::FinalMark | Pause::RefCount => unreachable!(),
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::Full | Pause::FinalMark => {
                if self.satb && pause == Pause::FinalMark {
                    self.span.clear_side_log_bits();
                    self.common
                        .schedule_unlog_bits_op(UnlogBitsOperation::BulkClear);
                }
                self.pacing.end_marking();
                // Mark termination: sweepgen += 2.  Mutator release (after this) returns and
                // sweeps the mutators' cached spans; sweep packets sweep the rest.
                self.span.release_after_mark();
                // Safety: the plan, and so the space, lives as long as MMTk.
                let span: &'static SpanSpace<VM> = unsafe { &*(&self.span as *const _) };
                // After `FinalMark`, Go's background sweeper runs concurrently with the
                // mutators (design §5.5).  VO bits must be cleared in a pause.
                let sweep_stage = if pause == Pause::FinalMark && !cfg!(feature = "vo_bit") {
                    WorkBucketStage::Concurrent
                } else {
                    WorkBucketStage::Release
                };
                span.schedule_sweep(sweep_stage);
                self.common.release(tls, true);
            }
            Pause::InitialMark | Pause::RefCount => unreachable!(),
        }
    }

    fn on_pause_end(&mut self, mmtk: &'static MMTK<VM>, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);

        let cycle_ended = match pause {
            Pause::InitialMark => {
                self.start_concurrent_marking(mmtk);
                self.pacing.limiter_transition(true);
                false
            }
            Pause::FinalMark if self.restarting.swap(false, Ordering::SeqCst) => {
                // Marking goes on.  The work found by `FinalMarkCheck` is in `Concurrent`.
                false
            }
            Pause::Full | Pause::FinalMark => true,
            Pause::RefCount => unreachable!(),
        };

        if cycle_ended {
            self.completed_cycles.fetch_add(1, Ordering::SeqCst);
            if pause == Pause::FinalMark {
                self.pacing.limiter_transition(false);
            }
            // Soft references are strong until the end of the cycle, including any marking
            // finished in the `FinalMark` pause.
            self.soft_refs_strong.store(false, Ordering::SeqCst);
            self.common.on_pause_end(tls);
            self.report_memory_stats();
            self.pacing.end_cycle(self.heap_marked_bytes());
            if pause == Pause::FinalMark {
                info!(
                    "Concurrent marking done after {} ragged flush round(s), {} restart(s), {} allocation stall(s), {} assist(s) ({} parked)",
                    self.ragged_rounds.load(Ordering::SeqCst),
                    self.restarts.load(Ordering::SeqCst),
                    self.stalls.load(Ordering::SeqCst),
                    self.assists.count.load(Ordering::Relaxed),
                    self.assists.parked_count.load(Ordering::Relaxed),
                );
            }
            mmtk.gc_trigger.policy.on_gc_end(mmtk);
        }
        self.stats.pause_ended(pause);
        info!("{:?} end", pause);
    }

    fn current_gc_may_move_object(&self) -> bool {
        false
    }

    fn get_used_pages(&self) -> usize {
        self.span.reserved_pages() + self.common.get_used_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn concurrent(&self) -> Option<&dyn ConcurrentPlan<VM = VM>> {
        Some(self)
    }
}

impl<VM: VMBinding> ConcurrentSpan<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let pacing = Pacing::new(&args.options);
        let stats = SpanStats::new(args.stats);
        let assists_enabled =
            *args.options.concurrent_span_assists && VM::VMCollection::supports_blocking_mutators();
        let satb = *args.options.concurrent_span_satb_barrier;
        // With the SATB barrier, the stacks must be scanned at the start of marking.
        let concurrent_stack_scan = !satb
            && !*args.options.concurrent_span_scan_stacks_in_pause
            && VM::VMScanning::supports_concurrent_stack_scanning();

        let log_bit_specs = if satb {
            crate::util::metadata::extract_side_metadata(&[*VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC])
        } else {
            vec![]
        };
        let mut global_side_metadata_specs = SideMetadataContext::new_global_specs(&log_bit_specs);
        SpanSpace::<VM>::extend_global_side_metadata_specs(&mut global_side_metadata_specs);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: if satb {
                &CONCURRENT_SPAN_SATB_CONSTRAINTS
            } else {
                &CONCURRENT_SPAN_CONSTRAINTS
            },
            global_side_metadata_specs,
        };

        // These buckets are not used by a non-moving plan.
        let scheduler = &plan_args.global_args.scheduler;
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SecondRoots].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::RefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::FinalizableForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::Compact].set_enabled(false);

        ConcurrentSpan {
            span: SpanSpace::new(plan_args.get_normal_space_args(
                "span",
                true,
                false,
                VMRequest::discontiguous(),
            )),
            common: CommonPlan::new(plan_args),
            current_pause: Atomic::new(None),
            previous_pause: Atomic::new(None),
            pacing,
            assists: Assists::default(),
            stats,
            completed_cycles: AtomicUsize::new(0),
            assists_enabled,
            marking: AtomicBool::new(false),
            mark_done: AtomicBool::new(false),
            marking_interrupted: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
            ragged_rounds: AtomicUsize::new(0),
            restarts: AtomicUsize::new(0),
            stalls: AtomicUsize::new(0),
            soft_refs_strong: AtomicBool::new(false),
            concurrent_stack_scan,
            satb,
        }
    }

    /// Return `true` if cycles mark concurrently.
    fn concurrent_marking_supported(&self) -> bool {
        !*self
            .base()
            .options
            .concurrent_span_disable_concurrent_marking
    }

    /// The pacer's live heap estimate (Go's `heapLive`), in bytes.  `SpanSpace` charges it at
    /// span refill, as Go does.  The other spaces (LOS, immortal) count their used pages.
    fn heap_live_bytes(&self) -> u64 {
        (self.span.heap_live()
            + crate::util::conversions::pages_to_bytes(self.common.get_used_pages())) as u64
    }

    /// The live heap found by the cycle that just ended (Go's `heapMarked`), in bytes: the bytes
    /// marked in `SpanSpace`, including objects allocated black, plus the other spaces' used
    /// pages after the release.
    /// Tell the pacer how much memory counts against the memory limit, for its memory-limit
    /// heap goal (Go keeps `mappedReady`, `heapFree` and `totalAlloc - totalFree` up to date).
    /// Everything MMTk has reserved is `mappedReady`; the free pages of SpanSpace's page regions
    /// are `heapFree`; `heap_live`, capped at the bytes in spans, stands in for the object bytes.
    /// What is left (free slots in spans beyond `heap_live`, and metadata) is non-heap memory to
    /// Go's pacer.
    fn report_memory_stats(&self) {
        use crate::policy::spanspace::classes::BYTES_IN_SPAN_PAGE;
        use crate::util::constants::BYTES_IN_PAGE;
        let mapped_ready = (self.get_reserved_pages() * BYTES_IN_PAGE) as u64;
        let span_mapped = (self.span.reserved_pages() * BYTES_IN_PAGE) as u64;
        let in_spans = (self.span.pages_in_use() * BYTES_IN_SPAN_PAGE) as u64;
        let heap_free = span_mapped.saturating_sub(in_spans);
        // Like `heap_live_bytes`, the other spaces' pages (large objects, for example) count as
        // heap, as Go's large objects are in heap spans.
        let common = crate::util::conversions::pages_to_bytes(self.common.get_used_pages()) as u64;
        let heap_alloc = (self.span.heap_live() as u64).min(in_spans) + common;
        self.pacing
            .set_memory_stats(mapped_ready, heap_free, heap_alloc);
    }

    fn heap_marked_bytes(&self) -> u64 {
        (self.span.bytes_marked()
            + crate::util::conversions::pages_to_bytes(self.common.get_used_pages())) as u64
    }

    fn set_ref_closure_buckets_enabled(&self, do_closure: bool) {
        let scheduler = &self.common.base.scheduler;
        scheduler.work_buckets[WorkBucketStage::VMRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::WeakRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::FinalRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::PhantomRefClosure].set_enabled(do_closure);
    }

    /// Go's sweep termination pause (`gcStart`, `runtime/mgc.go:835-930`).
    fn schedule_initial_mark(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.set_ref_closure_buckets_enabled(false);
        // An emergency cycle clears soft references (as Java requires before running out of
        // memory), so they are discovered as candidates.  Otherwise they are strong.
        self.soft_refs_strong.store(
            !self.base().global_state.is_emergency_collection(),
            Ordering::SeqCst,
        );
        let stop_mutators = if self.concurrent_stack_scan {
            // Only VM roots are scanned in the pause.  Stacks are scanned during marking.
            StopMutators::<ConcurrentSpanGCWorkContext<VM>>::new_no_scan_mutator_roots()
        } else {
            StopMutators::<ConcurrentSpanGCWorkContext<VM>>::new()
        };
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(stop_mutators);
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<ConcurrentSpanGCWorkContext<VM>>::new(self));
    }

    /// Go's mark termination pause (`runtime/mgc.go:1066-1156`).  The pause starts with
    /// [`FinalMarkCheck`], which schedules the rest unless marking restarts.
    fn schedule_final_mark(&'static self, scheduler: &GCWorkScheduler<VM>) {
        // Stop the mutators and flush their barrier buffers.  Do not scan roots.
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<ConcurrentSpanGCWorkContext<VM>>::new_no_scan_roots());
        scheduler.work_buckets[WorkBucketStage::Prepare].add(FinalMarkCheck);
    }

    /// Called by [`FinalMarkCheck`] at the start of a `FinalMark` pause.
    pub(super) fn final_mark_check(&'static self, mmtk: &'static MMTK<VM>) {
        let concurrent = &mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent];
        let interrupted = self.marking_interrupted.swap(false, Ordering::SeqCst);
        let found_work = !concurrent.is_empty();
        let max_restarts = *self.base().options.concurrent_span_max_mark_restarts;

        if found_work && !interrupted && self.restarts.load(Ordering::SeqCst) < max_restarts {
            // Go: "restart the world and resume concurrent mark".  The barrier stays on, and the
            // scheduler resumes the mutators into the work left in `Concurrent`.
            self.restarts.fetch_add(1, Ordering::SeqCst);
            self.stats.mark_restart();
            self.mark_done.store(false, Ordering::SeqCst);
            self.restarting.store(true, Ordering::SeqCst);
            info!(
                "FinalMark found {} packet(s) of marking work.  Restarting concurrent marking.",
                concurrent.len()
            );
            return;
        }

        if found_work {
            // Finish the marking in this pause.  That happens when the heap ran out during
            // marking, or after too many restarts.
            info!(
                "FinalMark finishes {} packet(s) of marking work in the pause (interrupted: {})",
                concurrent.len(),
                interrupted
            );
            mmtk.scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork]
                .bulk_add(concurrent.drain_all_packets());
        }

        // Marking is done.  New barrier flushes go to the `Closure` bucket of this pause.
        self.marking.store(false, Ordering::SeqCst);
        // Release the parked assists.  They wake up when the pause ends.
        if self.assists.release_all() {
            VM::VMCollection::wake_blocked_mutators();
        }
        self.set_marking_state(false);
        self.schedule_final_mark_release(mmtk.get_plan().base().scheduler.as_ref());
    }

    fn schedule_final_mark_release(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.set_ref_closure_buckets_enabled(true);
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<ConcurrentSpanGCWorkContext<VM>>::new(self));

        #[cfg(feature = "sanity")]
        {
            use crate::util::sanity::sanity_checker::ScheduleSanityGC;
            scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC::<Self>::new(self));
        }

        // Reference processing, as in `ConcurrentImmix`.
        // TODO: `SoftRefProcessing` and `Finalization` still trace in the pause (design §5.14).
        type RefTracePolicy<VM> =
            crate::plan::tracing::PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, RefEnqueue, SoftRefProcessing, WeakRefProcessing,
            };
            // If soft references were strong in this cycle, the binding discovered none, and
            // their referents have been traced already.
            if !self.soft_refs_strong.load(Ordering::SeqCst) {
                scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                    .add(SoftRefProcessing::<RefTracePolicy<VM>>::new());
            }
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }
        if !*self.base().options.no_finalizer {
            use crate::util::finalizable_processor::Finalization;
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<RefTracePolicy<VM>>::new());
        }
        scheduler.work_buckets[WorkBucketStage::VMRefClosure]
            .set_sentinel(Box::new(VMProcessWeakRefs::<RefTracePolicy<VM>>::new()));
    }

    /// At the end of `InitialMark`: turn on the barrier and allocate-black, and start the
    /// concurrent stack scan.
    fn start_concurrent_marking(&self, mmtk: &'static MMTK<VM>) {
        self.mark_done.store(false, Ordering::SeqCst);
        self.marking_interrupted.store(false, Ordering::SeqCst);
        self.ragged_rounds.store(0, Ordering::SeqCst);
        self.restarts.store(0, Ordering::SeqCst);
        self.stalls.store(0, Ordering::SeqCst);
        self.assists.reset_stats();
        self.marking.store(true, Ordering::SeqCst);
        self.set_marking_state(true);
        if !*self.base().options.no_finalizer {
            mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent]
                .add_no_notify(ScanFinalizableContents);
        }
        if self.concurrent_stack_scan {
            // The scheduler opens `Concurrent` after this pause because it is not empty.
            mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent]
                .add_no_notify(ConcurrentScanStacks::<ConcurrentSpanGCWorkContext<VM>>::new());
        }
    }

    /// Turn the barrier and allocate-black on or off.  Mutators are stopped.
    fn set_marking_state(&self, active: bool) {
        use crate::plan::global::HasSpaces;
        self.for_each_space(&mut |space: &dyn Space<VM>| {
            space.set_allocate_as_live(active);
        });
        if !self.satb {
            HybridBarrierType::<VM>::set_active(active);
        }
    }

    /// The number of ragged flush rounds and `FinalMark` restarts in the current (or last) cycle.
    #[cfg(test)]
    pub(crate) fn cycle_stats(&self) -> (usize, usize) {
        (
            self.ragged_rounds.load(Ordering::SeqCst),
            self.restarts.load(Ordering::SeqCst),
        )
    }

    /// The kind of the last pause that ended.
    #[cfg(test)]
    pub(crate) fn previous_pause(&self) -> Option<Pause> {
        self.previous_pause.load(Ordering::SeqCst)
    }

    /// The number of assists, and of parked assists, in the current (or last) cycle.
    #[cfg(test)]
    pub(crate) fn assist_stats(&self) -> (usize, usize) {
        (
            self.assists.count.load(Ordering::Relaxed),
            self.assists.parked_count.load(Ordering::Relaxed),
        )
    }

    /// The number of allocation stalls in the current (or last) cycle.
    #[cfg(test)]
    pub(crate) fn stall_count(&self) -> usize {
        self.stalls.load(Ordering::SeqCst)
    }

    /// A mutator whose allocation credit (`credit`, Go's `gcAssistBytes`) is negative asks to
    /// assist.  See [`crate::plan::concurrent::span::assist`].
    pub(super) fn assist(&self, tls: crate::util::VMThread, credit: &mut isize) {
        if !self.assists_enabled || !self.marking.load(Ordering::SeqCst) {
            *credit = 0;
            return;
        }
        if self.pacing.limiting() {
            // The GC is using too much CPU: do not assist (the debt stays).
            self.stats.assist_limited();
            return;
        }
        match self.assists.start(&self.pacing.pacer, *credit) {
            AssistOutcome::Paid(bytes) => {
                self.stats.assist(false);
                *credit += bytes
            }
            AssistOutcome::Park(request, bytes) => {
                self.stats.assist(true);
                // Lend the mutator's CPU to marking: activate one more GC worker.
                let scheduler = &self.common.base.scheduler;
                let threads = *self.base().options.threads;
                let active = scheduler.num_active_workers();
                if active < threads {
                    scheduler.set_active_workers(active + 1);
                    scheduler.work_buckets[WorkBucketStage::Concurrent].notify_all_workers();
                }
                let start = self.pacing.now_ns();
                VM::VMCollection::block_mutator_until(crate::util::VMMutatorThread(tls), &|| {
                    request.is_done() || !self.marking.load(Ordering::SeqCst)
                });
                self.assists.unparked();
                // Stage A counts the time parked as assist time.
                let parked = self.pacing.now_ns() - start;
                self.pacing.pacer.add_assist_time(parked);
                self.pacing.limiter.add_assist_time(parked);
                *credit = if self.marking.load(Ordering::SeqCst) {
                    *credit + bytes
                } else {
                    0
                };
            }
        }
    }

    /// Called by [`RaggedFlush`] when a round has flushed every mutator.
    pub(super) fn on_ragged_flush_done(&self, found_work: bool) {
        self.ragged_rounds.fetch_add(1, Ordering::SeqCst);
        self.stats.ragged_round();
        if !found_work {
            // Go: `gcMarkDoneFlushed == 0`.  When the last worker parks, `collection_required`
            // requests `FinalMark`.
            self.mark_done.store(true, Ordering::SeqCst);
        }
    }
}

impl<VM: VMBinding> ConcurrentPlan for ConcurrentSpan<VM> {
    fn concurrent_work_in_progress(&self) -> bool {
        self.marking.load(Ordering::SeqCst)
    }

    fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    fn on_concurrent_work_interrupted(&self) {
        self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
        if self.marking.load(Ordering::SeqCst) {
            // A pause was requested during concurrent marking (the heap is full and allocation
            // stalls are off).  `FinalMark` finishes the marking in the pause.
            self.marking_interrupted.store(true, Ordering::SeqCst);
        }
        // Otherwise background sweeping was interrupted by a new cycle.
    }

    fn concurrent_user_collection(&self) -> bool {
        *self.base().options.concurrent_span_concurrent_user_gc
            && self.concurrent_marking_supported()
    }

    fn completed_cycles(&self) -> usize {
        self.completed_cycles.load(Ordering::SeqCst)
    }

    fn shade_objects(&self, objects: &[crate::util::ObjectReference]) {
        if objects.is_empty() || !self.marking.load(Ordering::SeqCst) {
            return;
        }
        use crate::plan::concurrent::concurrent_marking_work::ProcessModBufSATB;
        let packet =
            ProcessModBufSATB::<VM, ConcurrentSpan<VM>, DEFAULT_TRACE>::new(objects.to_vec());
        let bucket = &self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent];
        if bucket.is_enabled() {
            bucket.add(packet);
        } else {
            // In a pause (`FinalMark` checks this bucket for work).
            bucket.add_no_notify(packet);
        }
    }

    fn is_marked_in_concurrent_trace(&self, object: crate::util::ObjectReference) -> bool {
        self.span.in_space(object) && self.span.is_marked_while_marking(object)
    }

    fn counts_scan_work(&self) -> bool {
        true
    }

    fn add_scan_work(&self, heap: usize, stack: usize, globals: usize) {
        self.pacing.add_scan_work(heap, stack, globals);
        // Pay the parked assists, or add to the background credit (Go: `gcFlushBgCredit`).
        let work = (heap + stack + globals) as i64;
        if work > 0
            && self.assists_enabled
            && self.assists.flush_bg_credit(&self.pacing.pacer, work)
        {
            VM::VMCollection::wake_blocked_mutators();
        }
    }

    fn soft_references_are_strong(&self) -> bool {
        self.soft_refs_strong.load(Ordering::SeqCst)
    }

    fn trace_objects_concurrently(
        &self,
        worker: &mut crate::scheduler::GCWorker<VM>,
        objects: Vec<crate::util::ObjectReference>,
        already_marked: bool,
    ) -> Option<Vec<crate::util::ObjectReference>> {
        // Green Tea span scanning (design §5.12), unless ablated.
        if !self.span.green_tea() {
            return Some(objects);
        }
        super::greentea::trace_objects(worker, objects, already_marked);
        None
    }

    fn stall_instead_of_pause(&self) -> bool {
        // Go has no stop-the-world collection: an allocation that cannot be satisfied during
        // marking waits for the cycle (design §5.11).
        // Once marking is done, the collection a mutator needs is `FinalMark` itself, so it
        // requests it instead.
        *self.base().options.concurrent_span_allocation_stalls
            && self.marking.load(Ordering::SeqCst)
            && !self.mark_done.load(Ordering::SeqCst)
    }

    fn on_allocation_stall(&self) {
        self.stalls.fetch_add(1, Ordering::SeqCst);
        self.stats.stall();
        // The mutator cannot use its CPU, so give it to marking: activate every GC worker.  The
        // scheduler goes back to `concurrent_threads` at the end of the next pause.
        let scheduler = &self.common.base.scheduler;
        let threads = *self.base().options.threads;
        if scheduler.num_active_workers() < threads {
            scheduler.set_active_workers(threads);
            scheduler.work_buckets[WorkBucketStage::Concurrent].notify_all_workers();
        }
    }

    fn on_allocation_stall_finished(&self, duration: std::time::Duration) {
        self.stats.stall_finished(duration);
    }

    fn on_concurrent_work_drained(&self) -> bool {
        if !self.marking.load(Ordering::SeqCst) {
            // Background sweeping is done.  Leave the concurrent phase without a pause.
            if self
                .base()
                .global_state
                .gc_status
                .set_concurrent_gc_finished()
            {
                VM::VMCollection::concurrent_work_finished_no_pause();
            }
            return false;
        }
        // No work is left to pay the parked assists with (Go: `gcMarkDone` wakes all assists).
        if self.assists.release_all() {
            VM::VMCollection::wake_blocked_mutators();
        }
        if self.mark_done.load(Ordering::SeqCst) {
            return false;
        }
        if VM::VMCollection::supports_concurrent_mutator_flush() {
            // Run a ragged flush round.
            self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent]
                .add_no_notify(RaggedFlush);
        } else {
            // The binding cannot flush mutators while they run.  Request `FinalMark`, which
            // checks the buffers with the mutators stopped.
            self.mark_done.store(true, Ordering::SeqCst);
        }
        true
    }
}
