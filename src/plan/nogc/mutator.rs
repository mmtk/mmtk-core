use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::nogc::NoGC;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::allocators::{ReservedAllocators, base_allocator_mapping, common_allocator_mapping};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::EnumMap;

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let reserved = ReservedAllocators { n_bump_pointer: 1, ..ReservedAllocators::default() };
        let mut map = if cfg!(feature = "nogc_common_plan") {
            common_allocator_mapping(reserved)
        } else {
            base_allocator_mapping(reserved)
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
        space_mapping: box vec![
            (
                AllocatorSelector::BumpPointer(0),
                &plan.downcast_ref::<NoGC<VM>>().unwrap().nogc_space,
            ),
            #[cfg(feature = "ro_space")]
            (AllocatorSelector::BumpPointer(2), &plan.base().ro_space),
            #[cfg(feature = "code_space")]
            (AllocatorSelector::BumpPointer(3), &plan.base().code_space),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(4),
                &plan.base().code_lo_space,
            ),
            #[cfg(feature = "nogc_common_plan")]
            (
                AllocatorSelector::BumpPointer(1),
                plan.common().get_immortal(),
            ),
            #[cfg(feature = "nogc_common_plan")]
            (AllocatorSelector::LargeObject(0), plan.common().get_los()),
        ],
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
