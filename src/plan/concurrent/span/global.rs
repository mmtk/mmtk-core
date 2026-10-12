use crate::plan::barriers::HybridBarrier;
use crate::plan::concurrent::barrier::HybridBarrierSemantics;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::span::gc_work::{
    ConcurrentSpanGCWorkContext, ConcurrentSpanSTWGCWorkContext, FinalMarkCheck, RaggedFlush,
};
use crate::plan::concurrent::span::mutator::ALLOCATOR_MAPPING;
use crate::plan::concurrent::span::pacing::Pacing;
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
use crate::policy::marksweepspace::native_ms::MarkSweepSpace;
use crate::policy::marksweepspace::native_ms::MAX_OBJECT_SIZE;
use crate::policy::space::Space;
use crate::scheduler::gc_work::{ConcurrentScanStacks, Prepare, Release, StopMutators};
use crate::scheduler::{GCWorkScheduler, WorkBucketStage};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::VMWorkerThread;
use crate::vm::{Collection, Scanning, VMBinding};
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
/// `Pause::Full` (stop-the-world) is only used when the space cannot mark concurrently, and
/// for user-requested collections.
#[derive(HasSpaces, PlanTraceObject)]
pub struct ConcurrentSpan<VM: VMBinding> {
    #[space]
    pub(in crate::plan) ms: MarkSweepSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
    current_pause: Atomic<Option<Pause>>,
    previous_pause: Atomic<Option<Pause>>,
    /// Decides when a cycle starts.
    pacing: Pacing,
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
}

/// Whether the space can mark concurrently: it must allocate black and must not sweep against
/// partial marks.  Native `MarkSweepSpace` does neither (design §4, gaps 1 and 2).
// TODO: This becomes `true` with `SpanSpace`.
const SPACE_SUPPORTS_CONCURRENT_MARKING: bool = false;

/// Lets a test run concurrent cycles on a space that cannot mark concurrently.  The test must
/// not allocate while marking.
#[cfg(test)]
pub(crate) static FORCE_CONCURRENT_MARKING_FOR_TEST: AtomicBool = AtomicBool::new(false);

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

impl<VM: VMBinding> Plan for ConcurrentSpan<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &CONCURRENT_SPAN_CONSTRAINTS
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

        // The heap size from the GC trigger is the hard limit.
        if self.base().collection_required(self, space_full) {
            return true;
        }
        // Otherwise the pacer decides (Go: `gcTrigger.test`, `runtime/mgc.go:706-725`).
        self.pacing.report_heap_live(self.heap_live_bytes());
        self.pacing.should_start_cycle()
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let pause = if self.marking.load(Ordering::SeqCst) {
            Pause::FinalMark
        } else if !self.concurrent_marking_supported()
            // TODO: Go's `runtime.GC()` runs a concurrent cycle and blocks the caller until it
            // ends (design §5.2).  For now a user-requested collection is stop-the-world.
            || self.base().global_state.is_user_triggered_collection()
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
            mmtk.gc_trigger.policy.on_gc_start(mmtk);
            self.pacing.report_heap_live(self.heap_live_bytes());
            self.pacing.start_cycle();
        } else {
            // The barrier buffers were flushed into `Concurrent` when the mutators stopped.  Keep
            // that bucket closed during the pause: `FinalMarkCheck` decides what to do with them.
            mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
        }
        info!("{:?} start", pause);
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        match self.current_pause().unwrap() {
            Pause::Full | Pause::InitialMark => {
                self.common.prepare(tls, true);
                self.ms.prepare(true);
            }
            Pause::FinalMark | Pause::RefCount => unreachable!(),
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        match self.current_pause().unwrap() {
            Pause::Full | Pause::FinalMark => {
                self.pacing.end_marking();
                self.ms.release();
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
            // Soft references are strong until the end of the cycle, including any marking
            // finished in the `FinalMark` pause.
            self.soft_refs_strong.store(false, Ordering::SeqCst);
            self.ms.end_of_gc();
            self.common.on_pause_end(tls);
            // TODO: Use the bytes marked once the space counts them.  The space's size after the
            // sweep over-counts the live heap by its fragmentation.
            self.pacing.end_cycle(self.heap_live_bytes());
            if pause == Pause::FinalMark {
                info!(
                    "Concurrent marking done after {} ragged flush round(s), {} restart(s) and {} allocation stall(s)",
                    self.ragged_rounds.load(Ordering::SeqCst),
                    self.restarts.load(Ordering::SeqCst),
                    self.stalls.load(Ordering::SeqCst)
                );
            }
            mmtk.gc_trigger.policy.on_gc_end(mmtk);
        }
        info!("{:?} end", pause);
    }

    fn current_gc_may_move_object(&self) -> bool {
        false
    }

    fn get_used_pages(&self) -> usize {
        self.ms.reserved_pages() + self.common.get_used_pages()
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
        let concurrent_stack_scan = !*args.options.concurrent_span_scan_stacks_in_pause
            && VM::VMScanning::supports_concurrent_stack_scanning();

        let mut global_side_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        MarkSweepSpace::<VM>::extend_global_side_metadata_specs(&mut global_side_metadata_specs);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &CONCURRENT_SPAN_CONSTRAINTS,
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
            ms: MarkSweepSpace::new(plan_args.get_normal_space_args(
                "span",
                true,
                false,
                VMRequest::discontiguous(),
            )),
            common: CommonPlan::new(plan_args),
            current_pause: Atomic::new(None),
            previous_pause: Atomic::new(None),
            pacing,
            marking: AtomicBool::new(false),
            mark_done: AtomicBool::new(false),
            marking_interrupted: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
            ragged_rounds: AtomicUsize::new(0),
            restarts: AtomicUsize::new(0),
            stalls: AtomicUsize::new(0),
            soft_refs_strong: AtomicBool::new(false),
            concurrent_stack_scan,
        }
    }

    /// Return `true` if cycles mark concurrently.
    fn concurrent_marking_supported(&self) -> bool {
        #[cfg(test)]
        if FORCE_CONCURRENT_MARKING_FOR_TEST.load(Ordering::SeqCst) {
            return true;
        }
        SPACE_SUPPORTS_CONCURRENT_MARKING
            && !*self
                .base()
                .options
                .concurrent_span_disable_concurrent_marking
    }

    /// The pacer's live heap estimate (Go's `heapLive`), in bytes.
    // TODO: Count it at span refill, as Go does, once SpanSpace exists.
    fn heap_live_bytes(&self) -> u64 {
        crate::util::conversions::pages_to_bytes(self.get_used_pages()) as u64
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
        self.marking.store(true, Ordering::SeqCst);
        self.set_marking_state(true);
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
        HybridBarrierType::<VM>::set_active(active);
    }

    /// The number of ragged flush rounds and `FinalMark` restarts in the current (or last) cycle.
    #[cfg(test)]
    pub(crate) fn cycle_stats(&self) -> (usize, usize) {
        (
            self.ragged_rounds.load(Ordering::SeqCst),
            self.restarts.load(Ordering::SeqCst),
        )
    }

    /// The number of allocation stalls in the current (or last) cycle.
    #[cfg(test)]
    pub(crate) fn stall_count(&self) -> usize {
        self.stalls.load(Ordering::SeqCst)
    }

    /// Called by [`RaggedFlush`] when a round has flushed every mutator.
    pub(super) fn on_ragged_flush_done(&self, found_work: bool) {
        self.ragged_rounds.fetch_add(1, Ordering::SeqCst);
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
        // A pause was requested during concurrent marking (the heap is full).  `FinalMark`
        // finishes the marking in the pause.
        self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
        self.marking_interrupted.store(true, Ordering::SeqCst);
    }

    fn soft_references_are_strong(&self) -> bool {
        self.soft_refs_strong.load(Ordering::SeqCst)
    }

    fn stall_instead_of_pause(&self) -> bool {
        // Go has no stop-the-world collection: an allocation that cannot be satisfied during
        // marking waits for the cycle (design §5.11).
        *self.base().options.concurrent_span_allocation_stalls
            && self.marking.load(Ordering::SeqCst)
    }

    fn on_allocation_stall(&self) {
        self.stalls.fetch_add(1, Ordering::SeqCst);
        // The mutator cannot use its CPU, so give it to marking: activate every GC worker.  The
        // scheduler goes back to `concurrent_threads` at the end of the next pause.
        let scheduler = &self.common.base.scheduler;
        let threads = *self.base().options.threads;
        if scheduler.num_active_workers() < threads {
            scheduler.set_active_workers(threads);
            scheduler.work_buckets[WorkBucketStage::Concurrent].notify_all_workers();
        }
    }

    fn on_concurrent_work_drained(&self) -> bool {
        if !self.marking.load(Ordering::SeqCst) || self.mark_done.load(Ordering::SeqCst) {
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
