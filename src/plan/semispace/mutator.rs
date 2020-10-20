use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorContext;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::policy::space::Space;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::alloc::LargeObjectAllocator;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;

use crate::plan::mutator_context::MutatorConfig;
use crate::plan::semispace::SemiSpace;
use crate::vm::VMBinding;
use downcast_rs::Downcast;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ss_collection_phase<VM: VMBinding>(
    mutator: &mut Mutator<VM, SemiSpace<VM>>,
    tls: OpaquePointer,
    phase: &Phase,
    primary: bool,
) {
    match phase {
        Phase::Release => {
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
        _ => {}
    }
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_ss_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    plan: &'static SemiSpace<VM>,
) -> Mutator<VM, SemiSpace<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), plan.tospace()),
            (
                AllocatorSelector::BumpPointer(1),
                plan.common.get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), plan.common.get_los()),
        ],
        collection_phase_func: &ss_collection_phase,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        mutator_tls,
        config,
        plan,
    }
}
