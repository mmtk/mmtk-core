use crate::plan::barriers::NoBarrier;
use crate::plan::marksweep::MarkSweep;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::mutator_context::SpaceMapping;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;

use enum_map::EnumMap;

#[cfg(feature = "malloc_mark_sweep")]
mod malloc_mark_sweep {
    use super::*;

    // Do nothing for malloc mark sweep (malloc allocator)

    pub fn ms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}
    pub fn ms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

    // malloc mark sweep uses 1 malloc allocator

    pub(crate) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
        n_malloc: 1,
        ..ReservedAllocators::DEFAULT
    };
    lazy_static! {
        pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
            let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
            map[AllocationSemantics::Default] = AllocatorSelector::Malloc(0);
            map
        };
    }
    pub(crate) fn create_space_mapping<VM: VMBinding>(
        plan: &'static dyn Plan<VM = VM>,
    ) -> Box<SpaceMapping<VM>> {
        let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
        Box::new({
            let mut vec =
                crate::plan::mutator_context::create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            vec.push((AllocatorSelector::Malloc(0), ms.ms_space()));
            vec
        })
    }
}

#[cfg(not(feature = "malloc_mark_sweep"))]
mod native_mark_sweep {
    use super::*;
    use crate::util::alloc::FreeListAllocator;

    fn get_freelist_allocator_mut<VM: VMBinding>(
        mutator: &mut Mutator<VM>,
    ) -> &mut FreeListAllocator<VM> {
        unsafe {
            mutator
                .allocators
                .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
        }
        .downcast_mut::<FreeListAllocator<VM>>()
        .unwrap()
    }

    // We forward calls to the allocator prepare and release

    #[cfg(not(feature = "malloc_mark_sweep"))]
    pub fn ms_mutator_prepare<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
        get_freelist_allocator_mut::<VM>(mutator).prepare();
    }

    #[cfg(not(feature = "malloc_mark_sweep"))]
    pub fn ms_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
        get_freelist_allocator_mut::<VM>(mutator).release();
    }

    // native mark sweep uses 1 free list allocator

    pub(crate) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
        n_free_list: 1,
        ..ReservedAllocators::DEFAULT
    };
    lazy_static! {
        pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
            let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
            map[AllocationSemantics::Default] = AllocatorSelector::FreeList(0);
            map
        };
    }
    pub(crate) fn create_space_mapping<VM: VMBinding>(
        plan: &'static dyn Plan<VM = VM>,
    ) -> Box<SpaceMapping<VM>> {
        let ms = plan.downcast_ref::<MarkSweep<VM>>().unwrap();
        Box::new({
            let mut vec =
                crate::plan::mutator_context::create_space_mapping(RESERVED_ALLOCATORS, true, plan);
            vec.push((AllocatorSelector::FreeList(0), ms.ms_space()));
            vec
        })
    }
}

#[cfg(feature = "malloc_mark_sweep")]
pub use malloc_mark_sweep::*;

#[cfg(not(feature = "malloc_mark_sweep"))]
pub use native_mark_sweep::*;

pub fn create_ms_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: create_space_mapping(mmtk.get_plan()),
        prepare_func: &ms_mutator_prepare,
        release_func: &ms_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, mmtk, &config.space_mapping),
        barrier: Box::new(NoBarrier),
        mutator_tls,
        config,
        plan: mmtk.get_plan(),
    }
}
