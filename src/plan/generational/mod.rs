use crate::plan::barriers::BarrierSelector;
use crate::plan::PlanConstraints;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::vm::VMBinding;
use crate::vm::ObjectModel;
use crate::util::{Address, ObjectReference};
use crate::plan::AllocationSemantics;

use std::sync::atomic::Ordering;

pub(super) mod global;
pub(super) mod gc_work;

pub mod copying;

/// # Barrier overhead measurement:
///  - Set `FULL_NURSERY_GC` to `true`.
/// ## 1. Baseline: No barrier
///  - Set `ACTIVE_BARRIER` to `BarrierSelector::NoBarrier`.
/// ## 2. Object barrier
///  - Set `ACTIVE_BARRIER` to `BarrierSelector::ObjectBarrier`.
pub const ACTIVE_BARRIER: BarrierSelector = BarrierSelector::ObjectBarrier;
/// Full heap collection as nursery GC.
pub const FULL_NURSERY_GC: bool = false;
/// Force object barrier never enters the slow-path.
/// If enabled,
///  - `FULL_NURSERY_GC` must be `true`.
///  - `ACTIVE_BARRIER` must be `ObjectBarrier`.
pub const NO_SLOW: bool = false;

pub const GEN_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    needs_log_bit: true,
    barrier: ACTIVE_BARRIER,
    max_non_los_default_alloc_bytes: crate::util::rust_util::min_of_usize(
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
        crate::util::options::NURSERY_SIZE,
    ),
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

pub fn generational_post_copy<VM: VMBinding>(obj: ObjectReference, _tib: Address, _bytes: usize, _semantics: AllocationSemantics) {
    crate::util::object_forwarding::clear_forwarding_bits::<VM>(obj);
    if !NO_SLOW && ACTIVE_BARRIER == BarrierSelector::ObjectBarrier {
        VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(obj, Ordering::SeqCst);
    }
}