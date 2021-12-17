pub(super) use super::super::ALLOCATOR_MAPPING;
use crate::plan::mutator_context::Mutator;
use crate::plan::AllocationSemantics;
use crate::util::alloc::BumpAllocator;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;

pub fn gencopy_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn gencopy_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // reset nursery allocator
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.reset();
}
