use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::{create_allocator_mapping, ReservedAllocators};
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::vm::VMBinding;
use enum_map::EnumMap;

/// Prepare mutator. Do nothing.
pub(super) fn pp_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

/// Release mutator. Do nothing.
pub(super) fn pp_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

pub(super) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_large_object: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::LargeObject(0);
        map
    };
}
