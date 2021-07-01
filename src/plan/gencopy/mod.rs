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
