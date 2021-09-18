//! Plan: generational immix

pub(in crate::plan) mod gc_work;
pub(in crate::plan) mod global;
pub(in crate::plan) mod mutator;

pub use self::global::GenImmix;

pub use self::global::GENIMMIX_CONSTRAINTS;
