use super::PageProtect;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::{
    create_allocator_mapping, create_space_mapping, ReservedAllocators,
};
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::vm::VMBinding;
use crate::MMTK;
use crate::{
    plan::barriers::NoBarrier,
    util::opaque_pointer::{VMMutatorThread, VMWorkerThread},
};
use enum_map::EnumMap;

/// Prepare mutator. Do nothing.
fn pp_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

/// Release mutator. Do nothing.
fn pp_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

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
            vec.push((
                AllocatorSelector::LargeObject(0),
                page.space.clone().into_dyn_space(),
            ));
            vec
        }),
        prepare_func: &pp_mutator_prepare,
        release_func: &pp_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, mmtk, &config.space_mapping),
        barrier: Box::new(NoBarrier),
        mutator_tls,
        config,
        plan: page,
    }
}
