use super::MarkCompact; // Add
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::Plan;
use enum_map::enum_map;
use enum_map::EnumMap;

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::LargeCode | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_markcompact_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let markcompact = plan.downcast_ref::<MarkCompact<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), markcompact.mc_space()),
            (
                AllocatorSelector::BumpPointer(1),
                markcompact.common.get_immortal(),
            ),
            (
                AllocatorSelector::LargeObject(0),
                markcompact.common.get_los(),
            ),
        ],

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
    // rebind the allocation bump pointer to the appropriate semispace
    // let bump_allocator = unsafe {
    //     mutator
    //         .allocators
    //         .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    // }
    // .downcast_mut::<BumpAllocator<VM>>()
    // .unwrap();
    // bump_allocator.rebind(
    //     mutator
    //         .plan
    //         .downcast_ref::<MarkCompact<VM>>()
    //         .unwrap()
    //         .mc_space(),
    // );
}
