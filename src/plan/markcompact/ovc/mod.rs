//! Plan: OVC (mark-compact using offset-vector bitmaps)

pub(in crate::plan) mod gc_work;
pub(in crate::plan) mod global;
pub(in crate::plan) mod mutator;

pub use self::global::OVC;
