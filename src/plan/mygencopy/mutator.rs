use super::{MyGenCopy, gc_works::MGCNurseryProcessEdges};
use crate::{MMTK, plan::barriers::{FieldRememberingBarrier, NoBarrier}, policy::copyspace::CopySpace};
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

// Called by schedule_collection
pub fn mgc_mutator_prepare<VM: VMBinding>(
    _mutator: &mut Mutator<MyGenCopy<VM>>,
    _tls: OpaquePointer,
) {
    // Do nothing
}


// Called by schedule_collection
pub fn mgc_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<MyGenCopy<VM>>,
    _tls: OpaquePointer,
) {

    // Reset the allocator

    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.rebind(Some(&mutator.plan.nursery));
}

//Maps an allocation type to an allocation selector
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

// Called by bind_mutator(), creates a mutator for a thread
pub fn create_mgc_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    mmtk: &'static MMTK<VM>,
) -> Mutator<MyGenCopy<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), &mmtk.plan.nursery),
            (AllocatorSelector::BumpPointer(1), &mmtk.plan.mature),
            (AllocatorSelector::LargeObject(0), mmtk.plan.common.get_los()),
        ],
        prepare_func: &mgc_mutator_prepare,
        release_func: &mgc_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &mmtk.plan, &config.space_mapping),
        barrier: box FieldRememberingBarrier::<MGCNurseryProcessEdges<VM>, CopySpace<VM>>::new(
            mmtk,
            &mmtk.plan.nursery,
        ),
        mutator_tls,
        config,
        plan: &mmtk.plan,
    }
}