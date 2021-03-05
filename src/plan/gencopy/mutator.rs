use super::gc_work::*;
use super::GenCopy;
use crate::plan::barriers::*;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn gencopy_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: OpaquePointer) {
    // Do nothing
}

pub fn gencopy_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: OpaquePointer) {
    // reset nursery allocator
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.reset();
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_gencopy_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let gencopy = mmtk.plan.downcast_ref::<GenCopy<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), &gencopy.nursery),
            (
                AllocatorSelector::BumpPointer(1),
                gencopy.common.get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), gencopy.common.get_los()),
        ],
        prepare_func: &gencopy_mutator_prepare,
        release_func: &gencopy_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &*mmtk.plan, &config.space_mapping),
        barrier: box ObjectRememberingBarrier::<GenCopyNurseryProcessEdges<VM>>::new(
            mmtk,
            super::LOGGING_META,
        ),
        mutator_tls,
        config,
        plan: gencopy,
    }
}
