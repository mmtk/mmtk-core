use super::MallocMS;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::{ FreeListAllocator };
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(
    _mutator: &mut Mutator<MallocMS<VM>>,
    _tls: OpaquePointer,
) {
    // Do nothing
}

pub fn ms_mutator_release<VM: VMBinding>(
    _mutator: &mut Mutator<MallocMS<VM>>,
    _tls: OpaquePointer,
) {
    // Do nothing
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::FreeList(0),
        AllocationType:: Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::FreeList(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}


pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    plan: &'static MallocMS<VM>,
) -> Mutator<MallocMS<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::FreeList(0), &plan.space), //we ignore space
        ],
        prepare_func: &ms_mutator_prepare,
        release_func: &ms_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
