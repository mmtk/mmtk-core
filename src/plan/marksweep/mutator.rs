use super::MarkSweep;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::allocators::Allocators;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::Plan;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: OpaquePointer) {
    // Do nothing
}

pub fn ms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: OpaquePointer) {
    // Do nothing
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::Malloc(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(0),
        AllocationType::Los => AllocatorSelector::LargeObject(0),

        // AllocationType::Default | AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly | AllocationType::Los => AllocatorSelector::Malloc(0),
    };
}

pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::Malloc(0), &ms.space),
            (AllocatorSelector::BumpPointer(0), ms.common.get_immortal()),
            (AllocatorSelector::LargeObject(0), ms.common.get_los()),
        ],
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
