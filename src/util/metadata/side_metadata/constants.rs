use crate::util::heap::layout::vm_layout::VMLayout;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
use crate::util::metadata::side_metadata::SideMetadataOffset;
use crate::util::Address;

// XXX: We updated the base address to start from the second 4Mb chunk for 32-bit architectures,
// as otherwise for side metadatas with a large `min_obj_size`, we were overlapping with system
// reserved addresses such as 0x0.
// XXXX: I updated the base address for 32 bit to 0x1000_0000. For what I tested on, the library
// and the malloc heap often starts at 0x800_0000. If we start the metadata from the second 4Mb chunk (i.e. the chunk `[0x40_0000, 0x80_0000)`),
// we won't be guaranteed enough space before 0x800_0000. For example, the VO bit is 1 bit per 4 bytes
// (1 word in 32bits), and it will take the address range of [0x40_000, 0x840_0000) which clashes with
// the library/heap. So I move this to 0x1000_0000.
// This is made public, as VM bingdings may need to use this.
#[cfg(target_pointer_width = "32")]
/// Global side metadata start address
pub const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0x1000_0000) };

// FIXME: The 64-bit base address is changed from 0x0600_0000_0000 to 0x0c00_0000_0000 so that it
// is less likely to overlap with any space.  But it does not solve the problem completely.
// If there are more spaces, it will still overlap with some spaces.
// See: https://github.com/mmtk/mmtk-core/issues/458
#[cfg(target_pointer_width = "64")]
/// Global side metadata start address
pub const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0c00_0000_0000usize) };

pub(crate) const GLOBAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS);

/// Base address of VO bit, public to VM bindings which may need to use this.
pub const VO_BIT_SIDE_METADATA_ADDR: Address =
    crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_ADDR;

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

/// The max bytes (in log2) that may be used for global side metadata.
pub(crate) const LOG_MAX_GLOBAL_SIDE_METADATA_SIZE: usize =
    VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
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
pub(super) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;

/// The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal global side metadata.
pub const GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS: Address =
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_address_for_contiguous();
/// The base offset for the global side metadata available to VM bindings.
pub const GLOBAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
pub const LOCAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();
