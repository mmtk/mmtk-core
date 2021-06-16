//! Plan: marksweep (currently using malloc as its freelist allocator)

mod gc_work;
mod global;
pub mod mutator;

pub use self::global::MarkSweep;
pub use self::global::MS_CONSTRAINTS;
