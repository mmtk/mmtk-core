use crate::plan::barriers::{HybridBarrier, SATBBarrier};
use crate::plan::concurrent::barrier::{HybridBarrierSemantics, SATBBarrierSemantics};
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::concurrent::Pause;
use crate::plan::mutator_context::common_prepare_func;
use crate::plan::mutator_context::common_release_func;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::create_space_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorBuilder;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::SpanAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::EnumMap;

type BarrierSemanticsType<VM> = HybridBarrierSemantics<VM, ConcurrentSpan<VM>, DEFAULT_TRACE>;
type SATBBarrierType<VM> = SATBBarrier<SATBBarrierSemantics<VM, ConcurrentSpan<VM>, DEFAULT_TRACE>>;

/// With the SATB barrier option, turn its weak-reference barrier on or off.
fn set_satb_weak_ref_barrier<VM: VMBinding>(mutator: &mut Mutator<VM>, enabled: bool) {
    if let Some(barrier) = mutator.barrier.downcast_mut::<SATBBarrierType<VM>>() {
        barrier.set_weak_ref_barrier_enabled(enabled);
    }
}

fn get_span_allocator_mut<VM: VMBinding>(mutator: &mut Mutator<VM>) -> &mut SpanAllocator<VM> {
    unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<SpanAllocator<VM>>()
    .unwrap()
}

pub fn concurrent_span_mutator_prepare<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    tls: VMWorkerThread,
) {
    get_span_allocator_mut::<VM>(mutator).prepare();
    if mutator.plan.concurrent().unwrap().current_pause() == Some(Pause::InitialMark) {
        set_satb_weak_ref_barrier(mutator, true);
    }
    common_prepare_func(mutator, tls);
}

pub fn concurrent_span_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    tls: VMWorkerThread,
) {
    // Return (and, if stale, sweep) the cached spans. Runs after the plan's release, which
    // advanced the space's sweepgen.
    get_span_allocator_mut::<VM>(mutator).release();
    set_satb_weak_ref_barrier(mutator, false);
    common_release_func(mutator, tls);
}

pub(in crate::plan) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_span: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::Span(0);
        map
    };
}

pub fn create_concurrent_span_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let plan = mmtk
        .get_plan()
        .downcast_ref::<ConcurrentSpan<VM>>()
        .unwrap();
    plan.span
        .set_assist_hook_once(|| Box::new(move |tls, credit| plan.assist(tls, credit)));
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            vec.push((AllocatorSelector::Span(0), &plan.span));
            vec
        }),
        prepare_func: &concurrent_span_mutator_prepare,
        release_func: &concurrent_span_mutator_release,
    };

    let builder = MutatorBuilder::new(mutator_tls, mmtk, config);
    if plan.satb {
        let mut mutator = builder
            .barrier(Box::new(SATBBarrierType::<VM>::new(
                SATBBarrierSemantics::new(mmtk, mutator_tls),
            )))
            .build();
        set_satb_weak_ref_barrier(
            &mut mutator,
            crate::plan::ConcurrentPlan::concurrent_work_in_progress(plan),
        );
        mutator
    } else {
        builder
            .barrier(Box::new(HybridBarrier::new(
                BarrierSemanticsType::<VM>::new(mmtk, mutator_tls),
            )))
            .build()
    }
}
