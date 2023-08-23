pub mod block;
pub mod defrag;
pub mod immixspace;
pub mod line;

pub use immixspace::*;

use crate::policy::immix::block::Block;
use crate::util::linear_scan::Region;

/// The max object size for immix: half of a block
pub const MAX_IMMIX_OBJECT_SIZE: usize = Block::BYTES >> 1;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Do we allow Immix to do defragmentation?
pub const DEFRAG: bool = !cfg!(feature = "immix_non_moving"); // defrag if we are allowed to move.

/// Make every GC a defragment GC. (for debugging)
pub const STRESS_DEFRAG: bool = false;

/// Mark every allocated block as defragmentation source before GC. (for debugging)
/// Set both this and `STRESS_DEFRAG` to true to make Immix move as many objects as possible.
pub const DEFRAG_EVERY_BLOCK: bool = false;

/// If Immix is used as a nursery space, do we prefer copy?
pub const PREFER_COPY_ON_NURSERY_GC: bool =
    !cfg!(feature = "immix_non_moving") && !cfg!(feature = "sticky_immix_non_moving_nursery"); // copy nursery objects if we are allowed to move.

/// In some cases/settings, Immix may never move objects.
/// Currently we only have two cases where we move objects: 1. defrag, 2. nursery copy.
/// If we do neither, we will not move objects.
/// If we have other reasons to move objects, we need to add them here.
pub const NEVER_MOVE_OBJECTS: bool = !DEFRAG && !PREFER_COPY_ON_NURSERY_GC;

/// Mark lines when scanning objects.
/// Otherwise, do it at mark time.
pub const MARK_LINE_AT_SCAN_TIME: bool = true;

macro_rules! validate {
    ($x: expr) => { assert!($x, stringify!($x)) };
    ($x: expr => $y: expr) => { if $x { assert!($y, stringify!($x implies $y)) } };
}

fn validate_features() {
    // Block-only immix cannot do defragmentation
    validate!(DEFRAG => !BLOCK_ONLY);
    // Number of lines in a block should not exceed BlockState::MARK_MARKED
    assert!(Block::LINES / 2 <= u8::MAX as usize - 2);
}
