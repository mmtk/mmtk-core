///! A marksweep space that allocates from malloc.

mod global;
pub mod metadata;

pub use global::*;
pub(crate) use metadata::is_alloced_by_malloc;
