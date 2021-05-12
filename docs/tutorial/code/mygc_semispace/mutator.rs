// ANCHOR: imports
use super::MyGC; // Add
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;
// Remove crate::plan::mygc::MyGC
// Remove mygc_mutator_noop
// ANCHOR_END: imports

// Add
pub fn mygc_mutator_prepare<VM: VMBinding>(
    _mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // Do nothing
}

// Add
// ANCHOR: release
pub fn mygc_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // rebind the allocation bump pointer to the appropriate semispace
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.rebind(
        mutator
            .plan
            .downcast_ref::<MyGC<VM>>()
            .unwrap()
            .tospace(),
    );
}
// ANCHOR_END: release

// Modify
// ANCHOR: allocator_mapping
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}
// ANCHOR_END: allocator_mapping

pub fn create_mygc_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static MyGC<VM>,
) -> Mutator<VM> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        // Modify
        // ANCHOR: space_mapping
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), plan.tospace()),
            (
                AllocatorSelector::BumpPointer(1),
                plan.common.get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), plan.common.get_los()),
        ],
        // ANCHOR_END: space_mapping
        prepare_func: &mygc_mutator_prepare, // Modify
        release_func: &mygc_mutator_release, // Modify
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
