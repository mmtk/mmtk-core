use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::nogc::NoGC;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::allocators::{ReservedAllocators, base_allocator_mapping, common_allocator_mapping, base_space_mapping, common_space_mapping};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::EnumMap;

const NOGC_RESERVED_ALLOCATOR: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 1,
    n_large_object: 0,
    n_malloc: 0
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = if cfg!(feature = "nogc_common_plan") {
            common_allocator_mapping(NOGC_RESERVED_ALLOCATOR)
        } else {
            base_allocator_mapping(NOGC_RESERVED_ALLOCATOR)
        };
        map[AllocationType::Default] = AllocatorSelector::BumpPointer(0);
        map
    };
}

pub fn nogc_mutator_noop<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    unreachable!();
}

pub fn create_nogc_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box {
            let mut vec = if cfg!(feature = "nogc_common_plan") {
                common_space_mapping(NOGC_RESERVED_ALLOCATOR, plan)
            } else {
                base_space_mapping(NOGC_RESERVED_ALLOCATOR, plan)
            };
            vec.push((
                AllocatorSelector::BumpPointer(0),
                &plan.downcast_ref::<NoGC<VM>>().unwrap().nogc_space,
            ));
            vec
        },
        prepare_func: &nogc_mutator_noop,
        release_func: &nogc_mutator_noop,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
