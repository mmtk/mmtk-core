use super::barrier::IncrementalMarkSweepBarrierSemantics;
use super::global::IncrementalMarkSweep;
use crate::plan::barriers::SATBBarrier;
use crate::plan::concurrent::Pause;
use crate::plan::mutator_context::{
    create_allocator_mapping, create_space_mapping, Mutator, MutatorBuilder, MutatorConfig,
    ReservedAllocators,
};
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::FreeListAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::EnumMap;

type BarrierType<VM> = SATBBarrier<IncrementalMarkSweepBarrierSemantics<VM>>;

const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_free_list: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::FreeList(0);
        map[AllocationSemantics::NonMoving] = AllocatorSelector::FreeList(0);
        map
    };
}

fn free_list<VM: VMBinding>(m: &mut Mutator<VM>) -> &mut FreeListAllocator<VM> {
    unsafe {
        m.allocators
            .get_allocator_mut(m.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<FreeListAllocator<VM>>()
    .unwrap()
}

// `common_prepare_func` and `common_release_func` are not called: they expect `NonMoving` to
// have its own allocator.
pub fn incremental_marksweep_mutator_prepare<VM: VMBinding>(
    m: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // Scheduled for Full and InitialMark.
    free_list(m).prepare();
    if m.plan.concurrent().unwrap().current_pause() == Some(Pause::InitialMark) {
        m.barrier
            .downcast_mut::<BarrierType<VM>>()
            .unwrap()
            .set_weak_ref_barrier_enabled(true);
    }
}

pub fn incremental_marksweep_mutator_release<VM: VMBinding>(
    m: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // Scheduled for Full and FinalMark.
    free_list(m).release();
    m.barrier
        .downcast_mut::<BarrierType<VM>>()
        .unwrap()
        .set_weak_ref_barrier_enabled(false);
}

pub fn create_incremental_marksweep_mutator<VM: VMBinding>(
    tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let plan = mmtk
        .get_plan()
        .downcast_ref::<IncrementalMarkSweep<VM>>()
        .unwrap();
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut v = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            v.push((AllocatorSelector::FreeList(0), &plan.ms));
            v
        }),
        prepare_func: &incremental_marksweep_mutator_prepare,
        release_func: &incremental_marksweep_mutator_release,
    };
    let mut m = MutatorBuilder::new(tls, mmtk, config)
        .barrier(Box::new(SATBBarrier::new(
            IncrementalMarkSweepBarrierSemantics::new(mmtk, tls),
        )))
        .build();
    m.barrier
        .downcast_mut::<BarrierType<VM>>()
        .unwrap()
        .set_weak_ref_barrier_enabled(plan.is_marking());
    m
}
