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

// STRESS COPYING: Set the following options so that Immix will copy as many objects as possible.
// Useful for debugging copying GC if you cannot use SemiSpace.
//
// | constant                  | when    | value   | comment                                                              |
// |---------------------------|---------|---------|----------------------------------------------------------------------|
// | `STRESS_DEFRAG`           | default | `false` | By default, Immix only does defrag GC when necessary.                |
// | `STRESS_DEFRAG`           | stress  | `true`  | Set to `true` to force every GC to be defrag GC.                     |
// |                           |         |         |                                                                      |
// | `DEFRAG_EVERY_BLOCK`      | default | `false` | By default, Immix only defrags the most heavily fragmented blocks.   |
// | `DEFRAG_EVERY_BLOCK`      | stress  | `true`  | Set to `true` to make every block a defrag source.                   |
// |                           |         |         |                                                                      |
// | `DEFRAG_HEADROOM_PERCENT` | default | `2`     | Immix stops copying when space exhausted.                            |
// | `DEFRAG_HEADROOM_PERCENT` | stress  | `50`    | Reserve enough headroom to copy all objects.  50% is like SemiSpace. |

/// Make every GC a defragment GC. (for debugging)
pub const STRESS_DEFRAG: bool = true;

/// Mark every allocated block as defragmentation source before GC. (for debugging)
pub const DEFRAG_EVERY_BLOCK: bool = true;

/// Percentage of heap size reserved for defragmentation.
/// According to [this paper](https://doi.org/10.1145/1375581.1375586), Immix works well with
/// headroom between 1% to 3% of the heap size.
pub const DEFRAG_HEADROOM_PERCENT: usize = 50;

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
