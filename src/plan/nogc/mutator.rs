use crate::plan::barriers::NoBarrier;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::nogc::NoGC;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal => AllocatorSelector::BumpPointer(1),
        AllocationType::ReadOnly => AllocatorSelector::BumpPointer(2),
        AllocationType::Code => AllocatorSelector::BumpPointer(3),
        AllocationType::LargeCode => AllocatorSelector::BumpPointer(4),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn nogc_mutator_noop<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    unreachable!();
}

pub fn create_nogc_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    plan: &'static dyn Plan<VM = VM>,
) -> Mutator<VM> {
    let nogc = plan.downcast_ref::<NoGC<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), &nogc.nogc_space),
            (AllocatorSelector::BumpPointer(1), nogc.common.get_immortal()),
            (AllocatorSelector::BumpPointer(2), &nogc.base().ro_space),
            (AllocatorSelector::BumpPointer(3), &nogc.base().code_space),
            (AllocatorSelector::BumpPointer(4), &nogc.base().code_lo_space),
            (AllocatorSelector::LargeObject(0), nogc.common.get_los()),
        ],
        prepare_func: &nogc_mutator_noop,
        release_func: &nogc_mutator_noop,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        barrier: box NoBarrier,
        mutator_tls,
        config,
        plan,
    }
}
