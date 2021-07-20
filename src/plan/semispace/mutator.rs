use super::SemiSpace;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::allocators::{ReservedAllocators, common_allocator_mapping};
use crate::util::alloc::BumpAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::EnumMap;

pub fn ss_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn ss_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // rebind the allocation bump pointer to the appropriate semispace
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.rebind(
        mutator
            .plan
            .downcast_ref::<SemiSpace<VM>>()
            .unwrap()
            .tospace(),
    );
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = common_allocator_mapping(ReservedAllocators { n_bump_pointer: 1, ..ReservedAllocators::default() });
        map[AllocationType::Default] = AllocatorSelector::BumpPointer(0);
        map
    };
}

pub fn create_ss_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let ss = plan.downcast_ref::<SemiSpace<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), ss.tospace()),
            (AllocatorSelector::BumpPointer(1), ss.common.get_immortal()),
            (AllocatorSelector::LargeObject(0), ss.common.get_los()),
            #[cfg(feature = "ro_space")]
            (AllocatorSelector::BumpPointer(2), &ss.common.base.ro_space),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(3),
                &ss.common.base.code_space,
            ),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(4),
                &ss.common.base.code_lo_space,
            ),
        ],
        prepare_func: &ss_mutator_prepare,
        release_func: &ss_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
