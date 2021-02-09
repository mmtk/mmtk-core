pub(super) mod gc_works;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::GenCopy;

pub const FULL_NURSERY_GC: bool = true;
pub const NO_SLOW: bool = true;

pub use self::global::GENCOPY_CONSTRAINTS;
