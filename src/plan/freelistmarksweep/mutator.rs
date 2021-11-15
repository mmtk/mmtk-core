use crate::plan::barriers::NoBarrier;
use crate::plan::freelistmarksweep::FreeListMarkSweep;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::Allocator;
use crate::util::alloc::FreeListAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::FreeList(0),
        AllocationType::Los | AllocationType::Immortal | AllocationType::LargeCode => AllocatorSelector::BumpPointer(0),
    };
}
pub fn flms_mutator_prepare<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let space =         mutator
    .plan
    .downcast_ref::<FreeListMarkSweep<VM>>()
    .unwrap()
    .ms_space();
    // eprintln!("mutator prepare");
}

pub fn flms_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let free_list_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<FreeListAllocator<VM>>()
    .unwrap();
    free_list_allocator.rebind(
        mutator
            .plan
            .downcast_ref::<FreeListMarkSweep<VM>>()
            .unwrap()
            .ms_space(),
    )
}

pub fn create_freelistmarksweep_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (
                AllocatorSelector::FreeList(0),
                &plan
                    .downcast_ref::<FreeListMarkSweep<VM>>()
                    .unwrap()
                    .ms_space,
            ),
            (
                AllocatorSelector::BumpPointer(0),
                plan
                    .downcast_ref::<FreeListMarkSweep<VM>>()
                    .unwrap()
                    .common()
                    .get_immortal(),
            ),
        ],
        prepare_func: &flms_mutator_prepare,
        release_func: &flms_mutator_release,
    };
    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
