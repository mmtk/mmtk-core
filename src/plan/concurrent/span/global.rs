use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::span::gc_work::ConcurrentSpanSTWGCWorkContext;
use crate::plan::concurrent::span::mutator::ALLOCATOR_MAPPING;
use crate::plan::concurrent::span::pacing::Pacing;
use crate::plan::concurrent::Pause;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::marksweepspace::native_ms::MarkSweepSpace;
use crate::policy::marksweepspace::native_ms::MAX_OBJECT_SIZE;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;

use atomic::Atomic;
use atomic::Ordering;
use enum_map::EnumMap;
use mmtk_macros::{HasSpaces, PlanTraceObject};

/// A port of Go's garbage collector: a concurrent, non-moving, non-generational mark-sweep
/// collector over a size-segregated span heap.  See `docs/design/go-gc-port.md`.
///
/// Phase 1 of the port: the plan runs on native `MarkSweepSpace` with stop-the-world
/// `Pause::Full` collections only.
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

impl<VM: VMBinding> Plan for ConcurrentSpan<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &CONCURRENT_SPAN_CONSTRAINTS
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        // The heap size from the GC trigger is the hard limit.
        if self.base().collection_required(self, space_full) {
            return true;
        }
        // Otherwise the pacer decides (Go: `gcTrigger.test`, `runtime/mgc.go:706-725`).
        self.pacing.report_heap_live(self.heap_live_bytes());
        self.pacing.should_start_cycle()
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let pause = Pause::Full;
        self.current_pause.store(Some(pause), Ordering::SeqCst);
        probe!(mmtk, concurrent_pause_determined, pause as usize);
        scheduler.schedule_common_work::<ConcurrentSpanSTWGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn on_pause_start(&self, mmtk: &'static MMTK<VM>) {
        let pause = self.current_pause().unwrap();
        // Every pause starts a new GC cycle, except `FinalMark`, which continues the cycle
        // started by the preceding `InitialMark` pause.
        if pause != Pause::FinalMark {
            mmtk.gc_trigger.policy.on_gc_start(mmtk);
            self.pacing.report_heap_live(self.heap_live_bytes());
            self.pacing.start_cycle();
        }
        info!("{:?} start", pause);
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        match self.current_pause().unwrap() {
            Pause::Full => {
                self.common.prepare(tls, true);
                self.ms.prepare(true);
            }
            pause => unreachable!("{pause:?} is not implemented yet"),
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        match self.current_pause().unwrap() {
            Pause::Full => {
                self.pacing.end_marking();
                self.ms.release();
                self.common.release(tls, true);
            }
            pause => unreachable!("{pause:?} is not implemented yet"),
        }
    }

    fn on_pause_end(&mut self, mmtk: &'static MMTK<VM>, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        self.ms.end_of_gc();
        self.common.on_pause_end(tls);
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
        // Every pause ends a GC cycle, except `InitialMark`.
        if pause != Pause::InitialMark {
            // TODO: Use the bytes marked once the space counts them.  The space's size after the
            // sweep over-counts the live heap by its fragmentation.
            self.pacing.end_cycle(self.heap_live_bytes());
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
    /// The pacer's live heap estimate (Go's `heapLive`), in bytes.
    // TODO: Count it at span refill, as Go does, once SpanSpace exists.
    fn heap_live_bytes(&self) -> u64 {
        crate::util::conversions::pages_to_bytes(self.get_used_pages()) as u64
    }

    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let pacing = Pacing::new(&args.options);
        let mut global_side_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        MarkSweepSpace::<VM>::extend_global_side_metadata_specs(&mut global_side_metadata_specs);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &CONCURRENT_SPAN_CONSTRAINTS,
            global_side_metadata_specs,
        };

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
        }
    }
}

impl<VM: VMBinding> ConcurrentPlan for ConcurrentSpan<VM> {
    fn concurrent_work_in_progress(&self) -> bool {
        false
    }

    fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    fn on_concurrent_work_interrupted(&self) {
        unreachable!("ConcurrentSpan does not schedule concurrent work yet");
    }

    fn on_concurrent_work_drained(&self) {}
}
