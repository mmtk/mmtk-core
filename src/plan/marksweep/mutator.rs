use super::MarkSweep;
use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::allocators::Allocators;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::Plan;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn ms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::Malloc(0),
        AllocationType::Immortal => AllocatorSelector::BumpPointer(0),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
        AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Code => AllocatorSelector::BumpPointer(2),
        AllocationType::LargeCode => AllocatorSelector::BumpPointer(3),
    };
}

pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::Malloc(0), ms.ms_space()),
            (
                AllocatorSelector::BumpPointer(0),
                ms.common().get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), ms.common().get_los()),
            #[cfg(feature = "ro_space")]
            (
                AllocatorSelector::BumpPointer(1),
                &ms.common().base.ro_space,
            ),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(2),
                &ms.common().base.code_space,
            ),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(3),
                &ms.common().base.code_lo_space,
            ),
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
