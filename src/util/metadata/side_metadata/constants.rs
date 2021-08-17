use crate::util::heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::metadata::side_metadata::SideMetadataOffset;
use crate::util::Address;

// This is currently not used in 32-bits targets, but ultimately it is required in 32-bits global side metadata. So, instead of guarding with target_pointer_width, I allow unused_imports for now.
#[allow(unused_imports)]
use super::metadata_address_range_size;

// Global side metadata start address

// XXX: We updated the base address to start from the second 4Mb chunk for 32-bit architectures,
// as otherwise for side metadatas with a large `min_obj_size`, we were overlapping with system
// reserved addresses such as 0x0.
#[cfg(target_pointer_width = "32")]
pub(crate) const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(BYTES_IN_CHUNK) };
#[cfg(target_pointer_width = "64")]
pub(crate) const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0600_0000_0000usize) };

pub(crate) const GLOBAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS);

/// This constant represents the worst-case ratio of source data size to global side metadata.
/// A value of 2 means the space required for global side metadata must be less than 1/4th of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

/// This constant represents the worst-case ratio of source data size to global+local side metadata.
/// A value of 1 means the space required for global+local side metadata must be less than 1/2nd of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

const LOG_MAX_GLOBAL_SIDE_METADATA_SIZE: usize =
    LOG_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
// TODO - we should check this limit somewhere
// pub(crate) const LOG_MAX_LOCAL_SIDE_METADATA_SIZE: usize =
//     1 << (LOG_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);

// Local side metadata start address

pub(crate) const LOCAL_SIDE_METADATA_BASE_ADDRESS: Address =
    GLOBAL_SIDE_METADATA_BASE_ADDRESS.add(1usize << LOG_MAX_GLOBAL_SIDE_METADATA_SIZE);

// Local side metadata start offset

#[cfg(target_pointer_width = "32")]
pub(crate) const LOCAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset = SideMetadataOffset::rel(0);
#[cfg(target_pointer_width = "64")]
pub(crate) const LOCAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::addr(LOCAL_SIDE_METADATA_BASE_ADDRESS);

#[cfg(target_pointer_width = "32")]
pub(super) const CHUNK_MASK: usize = (1 << LOG_BYTES_IN_CHUNK) - 1;

#[cfg(target_pointer_width = "32")]
pub(super) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;

// The constants _VM_BASE_ADDRESS depends on the side metadata we use inside mmtk-core.
// If we add any new side metadata internal to mmtk-core, we need to update this accordingly.

// TODO: We should think if it is possible to update this when we create a new side metadata spec.
// One issue is that these need to be constants. Possibly we need to use macros or custom build scripts.

// --------------------------------------------------
//
// Global Metadata
//
// MMTk reserved Global side metadata offsets:
//
//  1 - Global Allocation bit:  1 bit per object, used by MarkSweep OR when enabling the global_alloc_bit feature
//  2 - MarkSweep Active Chunk byte: 1 byte per chunk, used by malloc marksweep.
//
// --------------------------------------------------

/// The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal global side metadata.

pub const GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS: Address = GLOBAL_SIDE_METADATA_BASE_ADDRESS
    .add(metadata_address_range_size(
        &crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC,
    ))
    .add(metadata_address_range_size(
        &crate::policy::mallocspace::metadata::ACTIVE_CHUNK_METADATA_SPEC,
    ));

pub const GLOBAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS);

// --------------------------------------------------
// PolicySpecific Metadata
//
// MMTk reserved PolicySpecific side metadata offsets:
//
//  1 - MarkSweep Active Page byte:
//      -
//  2 - Immix line mark byte:
//      - Offset after MS Active Page byte
//  3 - Immix block defrag byte:
//      - Offset after Immix block defrag byte
//  4 - Immix block mark byte:
//      - Offset after Immix block mark byte
//  5 - Immix chumk-map mark byte:
//      - Offset after Immix chumk-map mark byte
//
// --------------------------------------------------

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
pub const LOCAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::layout_after(&crate::policy::immix::LAST_LOCAL_SIDE_METADATA);
