//! Plan: Lisp2 (mark-compact using the Lisp-2 compaction algorithm)

pub(in crate::plan) mod gc_work;
pub(in crate::plan) mod global;
pub(in crate::plan) mod mutator;

pub use self::global::Lisp2;

pub use self::global::LISP2_CONSTRAINTS;
