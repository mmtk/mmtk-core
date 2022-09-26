//! Plan: marksweep

mod gc_work;
mod global;
pub mod mutator;

pub use self::global::MarkSweep;
pub use self::global::MS_CONSTRAINTS;
