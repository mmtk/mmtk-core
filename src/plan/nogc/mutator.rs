use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::{create_allocator_mapping, ReservedAllocators};
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use enum_map::{enum_map, EnumMap};

/// We use three bump allocators when enabling nogc_multi_space.
pub(super) const MULTI_SPACE_RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 3,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    /// When nogc_multi_space is disabled, force all the allocation go to the default allocator and space.
    pub static ref ALLOCATOR_MAPPING_SINGLE_SPACE: EnumMap<AllocationSemantics, AllocatorSelector> = enum_map! {
        _ => AllocatorSelector::BumpPointer(0),
    };
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        if cfg!(feature = "nogc_multi_space") {
            let mut map = create_allocator_mapping(MULTI_SPACE_RESERVED_ALLOCATORS, false);
            map[AllocationSemantics::Default] = AllocatorSelector::BumpPointer(0);
            map[AllocationSemantics::Immortal] = AllocatorSelector::BumpPointer(1);
            map[AllocationSemantics::Los] = AllocatorSelector::BumpPointer(2);
            map
        } else {
            *ALLOCATOR_MAPPING_SINGLE_SPACE
        }
    };
}

pub fn nogc_mutator_noop<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    unreachable!();
}
