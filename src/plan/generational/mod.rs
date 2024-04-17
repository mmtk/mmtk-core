//! Generational plans

use enum_map::EnumMap;

use crate::plan::barriers::BarrierSelector;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::AllocationSemantics;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::util::alloc::AllocatorSelector;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::Plan;

use super::mutator_context::create_space_mapping;
use super::mutator_context::ReservedAllocators;

// Generational plans:

pub mod barrier;
/// Generational copying (GenCopy)
pub mod copying;
/// Generational immix (GenImmix)
pub mod immix;

// Common generational code

pub(super) mod gc_work;
pub(super) mod global;

/// # Barrier overhead measurement:
///  - Set `FULL_NURSERY_GC` to `true`.
/// ## 1. Baseline: No barrier
///  - Set `ACTIVE_BARRIER` to `BarrierSelector::NoBarrier`.
/// ## 2. Object barrier
///  - Set `ACTIVE_BARRIER` to `BarrierSelector::ObjectBarrier`.
pub const ACTIVE_BARRIER: BarrierSelector = BarrierSelector::ObjectBarrier;
/// Full heap collection as nursery GC.
pub const FULL_NURSERY_GC: bool = false;

/// Constraints for generational plans. Each generational plan should overwrite based on this constant.
pub const GEN_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    needs_log_bit: ACTIVE_BARRIER.equals(BarrierSelector::ObjectBarrier),
    barrier: ACTIVE_BARRIER,
    // We may trace duplicate edges in sticky immix (or any plan that uses object remembering barrier). See https://github.com/mmtk/mmtk-core/issues/743.
    may_trace_duplicate_edges: ACTIVE_BARRIER.equals(BarrierSelector::ObjectBarrier),
    max_non_los_default_alloc_bytes:
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
};

/// Create global side metadata specs for generational plans. This will call SideMetadataContext::new_global_specs().
/// So if a plan calls this, it should not call SideMetadataContext::new_global_specs() again.
pub fn new_generational_global_metadata_specs<VM: VMBinding>() -> Vec<SideMetadataSpec> {
    let specs = if ACTIVE_BARRIER == BarrierSelector::ObjectBarrier {
        crate::util::metadata::extract_side_metadata(&[*VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC])
    } else {
        vec![]
    };
    SideMetadataContext::new_global_specs(&specs)
}

const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_bump_pointer: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::BumpPointer(0);
        map
    };
}

fn create_gen_space_mapping<VM: VMBinding>(
    plan: &'static dyn Plan<VM = VM>,
    nursery: &'static CopySpace<VM>,
) -> Vec<(AllocatorSelector, &'static dyn Space<VM>)> {
    let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, plan);
    vec.push((AllocatorSelector::BumpPointer(0), nursery));
    vec
}
