use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::{
    create_allocator_mapping, create_space_mapping, ReservedAllocators,
};
use crate::plan::nogc::NoGC;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::EnumMap;

const NOGC_RESERVED_ALLOCATOR: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 3,
    n_large_object: 0,
    n_malloc: 0,
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = create_allocator_mapping(NOGC_RESERVED_ALLOCATOR, false);
        map[AllocationType::Default] = AllocatorSelector::BumpPointer(0);
        map[AllocationType::Immortal] = AllocatorSelector::BumpPointer(1);
        map[AllocationType::Los] = AllocatorSelector::BumpPointer(2);
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
            let mut vec = create_space_mapping(NOGC_RESERVED_ALLOCATOR, false, plan);
            vec.push((
                AllocatorSelector::BumpPointer(0),
                &plan.downcast_ref::<NoGC<VM>>().unwrap().nogc_space,
            ));
            vec.push((
                AllocatorSelector::BumpPointer(1),
                &plan.downcast_ref::<NoGC<VM>>().unwrap().immortal,
            ));
            vec.push((
                AllocatorSelector::BumpPointer(2),
                &plan.downcast_ref::<NoGC<VM>>().unwrap().los,
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
