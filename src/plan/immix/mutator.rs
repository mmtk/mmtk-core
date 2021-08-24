use super::Immix;
use crate::plan::barriers::FLBKind;
use crate::plan::barriers::FieldLoggingBarrier;
use crate::plan::barriers::ObjectRememberingBarrier;
use crate::plan::immix::gc_work::ImmixProcessEdges;
use crate::plan::immix::gc_work::TraceKind;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics as AllocationType;
use crate::plan::Plan;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::alloc::ImmixAllocator;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::{
    plan::barriers::NoBarrier,
    util::opaque_pointer::{VMMutatorThread, VMWorkerThread},
};
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn immix_mutator_prepare<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let immix_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<ImmixAllocator<VM>>()
    .unwrap();
    immix_allocator.reset();
}

pub fn immix_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    let immix_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default])
    }
    .downcast_mut::<ImmixAllocator<VM>>()
    .unwrap();
    immix_allocator.reset();
}

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default => AllocatorSelector::Immix(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::LargeCode | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(0),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_immix_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let immix = mmtk.plan.downcast_ref::<Immix<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::Immix(0), &immix.immix_space),
            (
                AllocatorSelector::BumpPointer(0),
                immix.common.get_immortal(),
            ),
            (AllocatorSelector::LargeObject(0), immix.common.get_los()),
        ],
        prepare_func: &immix_mutator_prepare,
        release_func: &immix_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &*mmtk.plan, &config.space_mapping),
        barrier: box FieldLoggingBarrier::<ImmixProcessEdges<VM, { TraceKind::Fast }>, { FLBKind::IU }>::new(
            mmtk,
            *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
        ),
        mutator_tls,
        config,
        plan: &*mmtk.plan,
    }
}
