use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::mutator_context::common_prepare_func;
use crate::plan::mutator_context::common_release_func;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::create_space_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorBuilder;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::FreeListAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::EnumMap;

fn get_freelist_allocator_mut<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
) -> &mut FreeListAllocator<VM> {
    unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<FreeListAllocator<VM>>()
    .unwrap()
}

pub fn concurrent_span_mutator_prepare<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    tls: VMWorkerThread,
) {
    get_freelist_allocator_mut::<VM>(mutator).prepare();
    common_prepare_func(mutator, tls);
}

pub fn concurrent_span_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    tls: VMWorkerThread,
) {
    get_freelist_allocator_mut::<VM>(mutator).release();
    common_release_func(mutator, tls);
}

pub(in crate::plan) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_free_list: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::FreeList(0);
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
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            vec.push((AllocatorSelector::FreeList(0), &plan.ms));
            vec
        }),
        prepare_func: &concurrent_span_mutator_prepare,
        release_func: &concurrent_span_mutator_release,
    };

    MutatorBuilder::new(mutator_tls, mmtk, config).build()
}
