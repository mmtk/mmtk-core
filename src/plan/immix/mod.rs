pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

use crate::BarrierSelector;

pub use self::global::Immix;
pub use self::global::IMMIX_CONSTRAINTS;

use super::barriers::FLBKind;

pub const ACTIVE_BARRIER: BarrierSelector = BarrierSelector::FieldLoggingBarrier;

pub const CONCURRENT_MARKING: bool = true;

pub const BARRIER_MEASUREMENT: bool = false;

pub const FLB_KIND: FLBKind = FLBKind::SATB;
