//! Plan: generational copying

pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::GenCopy;
pub use self::global::GENCOPY_CONSTRAINTS;
