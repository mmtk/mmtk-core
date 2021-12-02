use crate::plan::barriers::NoBarrier;
use crate::plan::marksweep::MarkSweep;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::MallocAllocator;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
#[cfg(not(feature="malloc"))]
use crate::util::alloc::FreeListAllocator;
use crate::policy::mallocspace::MallocSpace;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

#[cfg(feature="malloc")]
pub fn ms_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

#[cfg(not(feature="malloc"))]
pub fn ms_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<FreeListAllocator<VM>>()
    .unwrap();
    allocator.rebind(
        mutator
            .plan
            .downcast_ref::<MarkSweep<VM>>()
            .unwrap()
            .ms_space(),
    )
}

#[cfg(feature="malloc")]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::Malloc(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::LargeCode | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(0),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

#[cfg(not(feature="malloc"))]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::FreeList(0),
        AllocationType::Immortal | AllocationType::LargeCode => AllocatorSelector::BumpPointer(0),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
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
            #[cfg(feature="malloc")]
            (AllocatorSelector::Malloc(0), ms.ms_space()),
            #[cfg(not(feature="malloc"))]
            (AllocatorSelector::FreeList(0), ms.ms_space()),
            (AllocatorSelector::BumpPointer(0), ms.common().get_immortal()),
            (AllocatorSelector::LargeObject(0), ms.common().get_los()),
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