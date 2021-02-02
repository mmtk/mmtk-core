use super::MyGC; // Add
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;
// Remove crate::plan::mygc::MyGC
// Remove mygc_mutator_noop

// Add
pub fn mygc_mutator_prepare<VM: VMBinding>(
    _mutator: &mut Mutator<MyGC<VM>>,
    _tls: OpaquePointer,
) {
    // Do nothing
}

// Add
pub fn mygc_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<MyGC<VM>>,
    _tls: OpaquePointer,
) {
    // rebind the allocation bump pointer to the appropriate semispace
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.rebind(Some(mutator.plan.tospace()));
}

// Modify
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_mygc_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    plan: &'static MyGC<VM>,
) -> Mutator<MyGC<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        // Modify
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), plan.tospace()),
            (
                AllocatorSelector::BumpPointer(1),
                plan.common.get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), plan.common.get_los()),
        ],
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
