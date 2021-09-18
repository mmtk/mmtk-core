use super::gc_work::GenImmixCopyContext;
use crate::plan::barriers::ObjectRememberingBarrier;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::generational::immix::GenImmix;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::BumpAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;

use enum_map::enum_map;
use enum_map::EnumMap;

pub fn genimmix_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {}

pub fn genimmix_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // reset nursery allocator
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.reset();
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = enum_map! {
        AllocationSemantics::Default => AllocatorSelector::BumpPointer(0),
        AllocationSemantics::Immortal | AllocationSemantics::Code | AllocationSemantics::LargeCode | AllocationSemantics::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationSemantics::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_genimmix_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let genimmix = mmtk.plan.downcast_ref::<GenImmix<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), &genimmix.gen.nursery),
            (
                AllocatorSelector::BumpPointer(1),
                genimmix.gen.common.get_immortal(),
            ),
            (
                AllocatorSelector::LargeObject(0),
                genimmix.gen.common.get_los(),
            ),
        ],
        prepare_func: &genimmix_mutator_prepare,
        release_func: &genimmix_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &*mmtk.plan, &config.space_mapping),
        barrier:
            box ObjectRememberingBarrier::<GenNurseryProcessEdges<VM, GenImmixCopyContext<VM>>>::new(
                mmtk,
                *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
            ),
        mutator_tls,
        config,
        plan: genimmix,
    }
}
