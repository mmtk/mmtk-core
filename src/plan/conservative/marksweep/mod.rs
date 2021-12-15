//! Plan: conservative-marksweep (currently using malloc as its freelist allocator)

mod gc_work;
mod global;
pub mod mutator;

pub use self::global::ConservativeMarkSweep;
pub use self::global::CONSERVATIVE_MS_CONSTRAINTS;
