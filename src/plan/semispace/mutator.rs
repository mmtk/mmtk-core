use super::SemiSpace;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::enum_map;
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
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::LargeCode | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
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
