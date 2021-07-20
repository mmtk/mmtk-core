use super::MarkSweep;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::allocators::Allocators;
use crate::util::alloc::allocators::{
    common_allocator_mapping, common_space_mapping, ReservedAllocators,
};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::Plan;
use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn ms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

const MS_RESERVED_ALLOCATOR: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 0,
    n_large_object: 0,
    n_malloc: 1,
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = common_allocator_mapping(MS_RESERVED_ALLOCATOR);
        map[AllocationType::Default] = AllocatorSelector::Malloc(0);
        map
    };
}

pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box {
            let mut vec = common_space_mapping(MS_RESERVED_ALLOCATOR, plan);
            vec.push((AllocatorSelector::Malloc(0), ms.ms_space()));
            vec
        },
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
