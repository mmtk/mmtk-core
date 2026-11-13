//! Plan: semispace

pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::SS_CONSTRAINTS;
pub use self::global::SemiSpace;
