use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::MarkCompactAllocator;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::EnumMap;

pub(super) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
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
