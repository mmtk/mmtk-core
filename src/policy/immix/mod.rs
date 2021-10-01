pub mod block;
pub mod chunk;
pub mod defrag;
pub mod immixspace;
pub mod line;

pub use immixspace::*;

use crate::{
    policy::immix::block::Block,
    util::metadata::side_metadata::{SideMetadataOffset, SideMetadataSpec},
};

use self::chunk::ChunkMap;

use super::mallocspace::metadata::ACTIVE_PAGE_METADATA_SPEC;

/// The max object size for immix: half of a block
pub const MAX_IMMIX_OBJECT_SIZE: usize = Block::BYTES >> 1;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Opportunistic copying
pub const DEFRAG: bool = true;

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

/// The start of immix side metadata is after the last MallocSpace side metadata.
const IMMIX_LOCAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::layout_after(&ACTIVE_PAGE_METADATA_SPEC);

/// Immix's Last local side metadata. Used to calculate `LOCAL_SIDE_METADATA_VM_BASE_OFFSET`.
pub const LAST_LOCAL_SIDE_METADATA: SideMetadataSpec = ChunkMap::ALLOC_TABLE;
