use ::util::constants::*;

pub const MOVES_OBJECTS: bool = false;
pub const NUM_SPECIALIZED_SCANS: usize = 0;
// The two consts below need to be consistent
pub const NEEDS_LOG_BIT_IN_HEADER: bool = false;
pub const NEEDS_LOG_BIT_IN_HEADER_NUM: usize = 0;

pub const NEEDS_LINEAR_SCAN: bool = SUPPORT_CARD_SCANNING || LAZY_SWEEP;
pub const NEEDS_CONCURRENT_WORKERS: bool = false;

pub const GENERATE_GC_TRACE: bool = false;