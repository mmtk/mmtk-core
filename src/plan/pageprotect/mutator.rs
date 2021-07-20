use super::PageProtect;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::allocators::{ReservedAllocators, common_allocator_mapping};
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

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = {
        let mut map = common_allocator_mapping(ReservedAllocators { n_large_object: 1, ..ReservedAllocators::default() });
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
        space_mapping: box vec![
            (AllocatorSelector::LargeObject(0), &page.space),
            (
                AllocatorSelector::BumpPointer(0),
                plan.common().get_immortal(),
            ),
            #[cfg(feature = "ro_space")]
            (AllocatorSelector::BumpPointer(1), &plan.base().ro_space),
            #[cfg(feature = "code_space")]
            (AllocatorSelector::BumpPointer(2), &plan.base().code_space),
            #[cfg(feature = "code_space")]
            (
                AllocatorSelector::BumpPointer(3),
                &plan.base().code_lo_space,
            ),
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
