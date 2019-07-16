//pub use ::plan::plan_constraints::{NEEDS_LOG_BIT_IN_HEADER, NEEDS_LOG_BIT_IN_HEADER_NUM};
pub use ::plan::plan_constraints::*;

pub const MOVES_OBJECTS: bool = true;
pub const GC_HEADER_BITS: usize = 2;
pub const GC_HEADER_WORDS: usize = 0;
pub const NUM_SPECIALIZED_SCANS: usize = 1;
pub const NEEDS_CONCURRENT_WORKERS: bool = false;