use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::freelistmarksweep::FreeListMarkSweep;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::FreeList(0),
        AllocationType::Los | AllocationType::Immortal => AllocatorSelector::BumpPointer(0),
    };
}
pub fn flms_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn flms_mutator_release<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn create_freelistmarksweep_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![(
            AllocatorSelector::FreeList(0),
            &plan.downcast_ref::<FreeListMarkSweep<VM>>().unwrap().ms_space,
        ),
        (
            AllocatorSelector::BumpPointer(0),
            &plan.downcast_ref::<FreeListMarkSweep<VM>>().unwrap().im_space,
        )],
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
