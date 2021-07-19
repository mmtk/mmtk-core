//! Plan: nogc (allocation-only)

pub(super) mod global;
pub(super) mod mutator;

pub use self::global::FreeListMarkSweep;
pub use self::global::FLMS_CONSTRAINTS;
