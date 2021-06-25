//! Plan: pageprotect
//!
//! Allocate each object on a separate page and protect the memory on release.
//! This GC is commonly used for debugging purposes.

pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::PageProtect;
pub use self::global::CONSTRAINTS as PP_CONSTRAINTS;