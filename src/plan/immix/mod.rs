pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::Immix;

pub const CONCURRENT_MARKING: bool = false;

pub const BARRIER_MEASUREMENT: bool = true;
