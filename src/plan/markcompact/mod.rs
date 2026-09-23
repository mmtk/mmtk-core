//! Mark-compact plans.

/// Mark-compact using the Lisp-2 compaction algorithm
pub mod lisp2;
/// Mark-compact using offset-vector bitmaps (OVC)
pub mod ovc;

pub use lisp2::LISP2_CONSTRAINTS;
