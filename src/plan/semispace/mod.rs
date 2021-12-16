//! Plan: semispace

pub mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::SemiSpace;
pub use self::global::SS_CONSTRAINTS;
