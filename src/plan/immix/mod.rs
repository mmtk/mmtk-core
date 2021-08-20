pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

use crate::BarrierSelector;

pub use self::global::Immix;
pub use self::global::IMMIX_CONSTRAINTS;

pub const ACTIVE_BARRIER: BarrierSelector = BarrierSelector::FieldLoggingBarrier;

pub const CONCURRENT_MARKING: bool = true;