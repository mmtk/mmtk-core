use super::MarkCompact; // Add
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::create_space_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::MarkCompactAllocator;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::Plan;
use enum_map::EnumMap;

const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_mark_compact: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::MarkCompact(0);
        map
    };
}

pub fn create_markcompact_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let markcompact = plan.downcast_ref::<MarkCompact<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box {
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            vec.push((AllocatorSelector::MarkCompact(0), markcompact.mc_space()));
            vec
        },
        prepare_func: &markcompact_mutator_prepare,
        release_func: &markcompact_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}

pub fn markcompact_mutator_prepare<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
}

pub fn markcompact_mutator_release<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // reset the thread-local allocation bump pointer
    let markcompact_allocator = unsafe {
        _mutator
            .allocators
            .get_allocator_mut(_mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<MarkCompactAllocator<VM>>()
    .unwrap();
    markcompact_allocator.reset();
}
