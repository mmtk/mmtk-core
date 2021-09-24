//! Plan: nogc (allocation-only)
pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::MarkCompact;
pub use self::global::MARKCOMPACT_CONSTRAINTS;
