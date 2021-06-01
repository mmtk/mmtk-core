//! Plan: generational copying

pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::GenCopy;
use super::barriers::BarrierSelector;

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

pub use self::global::GENCOPY_CONSTRAINTS;

#[cfg(feature = "side_gc_header")]
use crate::util::gc_byte;
#[cfg(feature = "side_gc_header")]
use crate::util::metadata::side_metadata_address_range_size;
use crate::util::metadata::{MetadataScope, MetadataSpec, GLOBAL_SIDE_METADATA_BASE_ADDRESS};

const LOGGING_META: MetadataSpec = MetadataSpec {
    is_on_side: true,
    scope: MetadataScope::Global,
    #[cfg(not(feature = "side_gc_header"))]
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    #[cfg(feature = "side_gc_header")]
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        + side_metadata_address_range_size(gc_byte::SIDE_GC_BYTE_SPEC),
    num_of_bits: 1,
    log_min_obj_size: 3,
};
