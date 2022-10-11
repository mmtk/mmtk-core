use crate::plan::barriers::NoBarrier;
use crate::plan::marksweep::MarkSweep;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::create_space_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
#[cfg(not(feature = "malloc_mark_sweep"))]
use crate::util::alloc::FreeListAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;

use enum_map::EnumMap;

pub fn ms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

#[cfg(feature = "malloc_mark_sweep")]
pub fn ms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

#[cfg(not(feature = "malloc_mark_sweep"))]
pub fn ms_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<FreeListAllocator<VM>>()
    .unwrap();
    allocator.reset();
}

const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_malloc: 1,
    ..ReservedAllocators::DEFAULT
};

#[cfg(feature = "malloc_mark_sweep")]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::Malloc(0);
        map
    };
}

#[cfg(not(feature = "malloc_mark_sweep"))]
lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::FreeList(0);
        map
    };
}

pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            #[cfg(feature = "malloc_mark_sweep")]
            vec.push((AllocatorSelector::Malloc(0), ms.ms_space()));
            #[cfg(not(feature = "malloc_mark_sweep"))]
            vec.push((AllocatorSelector::FreeList(0), ms.ms_space()));
            vec
        }),
        prepare_func: &ms_mutator_prepare,
        release_func: &ms_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: Box::new(NoBarrier),
        mutator_tls,
        config,
        plan,
    }
}
