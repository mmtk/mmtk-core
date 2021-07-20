use super::PageProtect;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::{
    create_allocator_mapping, create_space_mapping, ReservedAllocators,
};
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::vm::VMBinding;
use crate::{
    plan::barriers::NoBarrier,
    util::opaque_pointer::{VMMutatorThread, VMWorkerThread},
};
use enum_map::EnumMap;

/// Prepare mutator. Do nothing.
fn pp_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

/// Release mutator. Do nothing.
fn pp_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

const PP_RESERVED_ALLOCATOR: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 0,
    n_large_object: 1,
    n_malloc: 0,
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = create_allocator_mapping(PP_RESERVED_ALLOCATOR, true);
        map[AllocationType::Default] = AllocatorSelector::LargeObject(0);
        map
    };
}

/// Create a mutator instance.
/// Every object is allocated to LOS.
pub fn create_pp_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let page = plan.downcast_ref::<PageProtect<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box {
            let mut vec = create_space_mapping(PP_RESERVED_ALLOCATOR, true, plan);
            vec.push((AllocatorSelector::LargeObject(0), &page.space));
            vec
        },
        prepare_func: &pp_mutator_prepare,
        release_func: &pp_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
