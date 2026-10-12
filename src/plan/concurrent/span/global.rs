use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::span::gc_work::ConcurrentSpanSTWGCWorkContext;
use crate::plan::concurrent::span::mutator::ALLOCATOR_MAPPING;
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
}

/// The plan constraints for `ConcurrentSpan`.
pub const CONCURRENT_SPAN_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    max_non_los_default_alloc_bytes: MAX_OBJECT_SIZE,
    may_trace_duplicate_edges: true,
    // Mutators flush their allocation caches in prepare and release.
    needs_prepare_mutator: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for ConcurrentSpan<VM> {
    fn constraints(&self) -> &'static PlanConstraints {
        &CONCURRENT_SPAN_CONSTRAINTS
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
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
