use super::PageProtect;
use crate::plan::mutator_context::common_prepare_func;
use crate::plan::mutator_context::common_release_func;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorBuilder;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::{
    create_allocator_mapping, create_space_mapping, ReservedAllocators,
};
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::opaque_pointer::VMMutatorThread;
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::EnumMap;

const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
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

/// Create a mutator instance.
/// Every object is allocated to LOS.
pub fn create_pp_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let page = mmtk.get_plan().downcast_ref::<PageProtect<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, page);
            vec.push((AllocatorSelector::LargeObject(0), &page.space));
            vec
        }),
        prepare_func: &common_prepare_func,
        release_func: &common_release_func,
    };

    let builder = MutatorBuilder::new(mutator_tls, mmtk, config);
    builder.build()
}
