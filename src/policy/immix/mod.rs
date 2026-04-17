pub mod block;
pub mod block_allocation;
pub mod defrag;
pub mod immixspace;
pub mod line;

pub use immixspace::*;

use crate::policy::immix::block::Block;

/// The max object size for immix: half of a block
pub const MAX_IMMIX_OBJECT_SIZE: usize = Block::BYTES;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Mark lines when scanning objects.
/// Otherwise, do it at mark time.
pub const MARK_LINE_AT_SCAN_TIME: bool = true;
