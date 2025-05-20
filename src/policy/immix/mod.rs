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

// STRESS COPYING: Set the feature 'immix_stress_copying' so that Immix will copy as many objects as possible.
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
pub const STRESS_DEFRAG: bool = cfg!(feature = "immix_stress_defrag");

/// Mark every allocated block as defragmentation source before GC. (for debugging)
pub const DEFRAG_EVERY_BLOCK: bool = cfg!(feature = "immix_defrag_every_block");

/// Mark lines when scanning objects.
/// Otherwise, do it at mark time.
pub const MARK_LINE_AT_SCAN_TIME: bool = true;
