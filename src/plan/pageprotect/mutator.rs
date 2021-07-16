use super::PageProtect;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::vm::VMBinding;
use crate::{
    plan::barriers::NoBarrier,
    util::opaque_pointer::{VMMutatorThread, VMWorkerThread},
};
use enum_map::enum_map;
use enum_map::EnumMap;

/// Prepare mutator. Do nothing.
fn pp_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

/// Release mutator. Do nothing.
fn pp_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

#[cfg(not(feature = "force_vm_spaces"))]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default | AllocationType::Los | AllocationType::Immortal | AllocationType::Code | AllocationType::LargeCode | AllocationType::ReadOnly  => AllocatorSelector::LargeObject(0),
    };
}

#[cfg(feature = "force_vm_spaces")]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::LargeObject(0),
        AllocationType::Immortal => AllocatorSelector::BumpPointer(0),
        AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Code => AllocatorSelector::BumpPointer(2),
        AllocationType::LargeCode => AllocatorSelector::BumpPointer(3),
        AllocationType::Los => AllocatorSelector::LargeObject(1),
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
        space_mapping: box vec![
            (AllocatorSelector::LargeObject(0), &page.space),
            #[cfg(feature = "force_vm_spaces")]
            (
                AllocatorSelector::BumpPointer(0),
                plan.common().get_immortal(),
            ),
            #[cfg(all(feature = "force_vm_spaces", feature = "ro_space"))]
            (AllocatorSelector::BumpPointer(1), &plan.base().ro_space),
            #[cfg(all(feature = "force_vm_spaces", feature = "code_space"))]
            (AllocatorSelector::BumpPointer(2), &plan.base().code_space),
            #[cfg(all(feature = "force_vm_spaces", feature = "code_space"))]
            (
                AllocatorSelector::BumpPointer(3),
                &plan.base().code_lo_space,
            ),
            #[cfg(feature = "force_vm_spaces")]
            (AllocatorSelector::LargeObject(1), plan.common().get_los()),
        ],
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
