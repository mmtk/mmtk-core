pub(crate) mod block;
pub(crate) mod chunks;
mod global;
pub mod metadata;
mod chunks;
mod block;

pub use global::*;

use crate::util::metadata::side_metadata::{SideMetadataOffset, SideMetadataSpec};

use self::{block::Block, chunks::ChunkMap};

use super::immix::IMMIX_LAST_LOCAL_SIDE_METADATA;


/// The start of immix side metadata is after the last MallocSpace side metadata.
const MARKSWEEP_LOCAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::layout_after(&IMMIX_LAST_LOCAL_SIDE_METADATA);

/// Immix's Last local side metadata. Used to calculate `LOCAL_SIDE_METADATA_VM_BASE_OFFSET`.
pub const LAST_LOCAL_SIDE_METADATA: SideMetadataSpec = ChunkMap::ALLOC_TABLE;
