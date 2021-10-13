use super::SideMetadataSpec;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::layout::Mmapper;
#[cfg(target_pointer_width = "32")]
use crate::util::metadata::side_metadata::address_to_chunked_meta_address;
use crate::util::Address;
use crate::util::{
    constants::{BITS_IN_WORD, BYTES_IN_PAGE, LOG_BITS_IN_BYTE},
    heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE,
};
use crate::MMAPPER;
use std::io::Result;

/// Performs address translation in contiguous metadata spaces (e.g. global and policy-specific in 64-bits, and global in 32-bits)
#[inline(always)]
pub(crate) fn address_to_contiguous_meta_address(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let log_bits_num = metadata_spec.log_num_of_bits as i32;
    let log_bytes_in_region = metadata_spec.log_bytes_in_region as usize;

    let rshift = (LOG_BITS_IN_BYTE as i32) - log_bits_num;

    if rshift >= 0 {
        metadata_spec.get_absolute_offset() + ((data_addr >> log_bytes_in_region) >> rshift)
    } else {
        metadata_spec.get_absolute_offset() + ((data_addr >> log_bytes_in_region) << (-rshift))
    }
}

/// Unmaps the specified metadata range, or panics.
#[cfg(test)]
pub(super) fn ensure_munmap_metadata(start: Address, size: usize) {
    use crate::util::memory;
    trace!("ensure_munmap_metadata({}, 0x{:x})", start, size);

    assert!(memory::munmap(start, size).is_ok())
}

/// Unmaps a metadata space (`spec`) for the specified data address range (`start` and `size`)
/// Returns the size in bytes that get munmapped.
#[cfg(test)]
pub(crate) fn ensure_munmap_contiguos_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
) -> usize {
    // nearest page-aligned starting address
    let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
    // nearest page-aligned ending address
    let mmap_size =
        address_to_meta_address(spec, start + size).align_up(BYTES_IN_PAGE) - mmap_start;
    if mmap_size > 0 {
        ensure_munmap_metadata(mmap_start, mmap_size);
    }
    mmap_size
}

/// Tries to mmap the metadata space (`spec`) for the specified data address range (`start` and `size`).
/// Setting `no_reserve` to true means the function will only map address range, without reserving swap-space/physical memory.
/// Returns the size in bytes that gets mmapped in the function if success.
pub(crate) fn try_mmap_contiguous_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
    no_reserve: bool,
) -> Result<usize> {
    debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
    debug_assert!(size % BYTES_IN_PAGE == 0);

    // nearest page-aligned starting address
    let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
    // nearest page-aligned ending address
    let mmap_size =
        address_to_meta_address(spec, start + size).align_up(BYTES_IN_PAGE) - mmap_start;
    if mmap_size > 0 {
        if !no_reserve {
            MMAPPER.ensure_mapped(mmap_start, mmap_size >> LOG_BYTES_IN_PAGE)
        } else {
            MMAPPER.quarantine_address_range(mmap_start, mmap_size >> LOG_BYTES_IN_PAGE)
        }
        .map(|_| mmap_size)
    } else {
        Ok(0)
    }
}

/// Performs the translation of data address (`data_addr`) to metadata address for the specified metadata (`metadata_spec`).
#[inline(always)]
pub(crate) fn address_to_meta_address(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
) -> Address {
    #[cfg(target_pointer_width = "32")]
    let res = {
        if metadata_spec.is_global {
            address_to_contiguous_meta_address(metadata_spec, data_addr)
        } else {
            address_to_chunked_meta_address(metadata_spec, data_addr)
        }
    };
    #[cfg(target_pointer_width = "64")]
    let res = { address_to_contiguous_meta_address(metadata_spec, data_addr) };

    trace!(
        "address_to_meta_address({:?}, addr: {}) -> 0x{:x}",
        metadata_spec,
        data_addr,
        res
    );

    res
}

pub(crate) const fn addr_rshift(metadata_spec: &SideMetadataSpec) -> i32 {
    ((LOG_BITS_IN_BYTE as usize) + metadata_spec.log_bytes_in_region
        - (metadata_spec.log_num_of_bits)) as i32
}

#[allow(dead_code)]
#[inline(always)]
pub const fn metadata_address_range_size(metadata_spec: &SideMetadataSpec) -> usize {
    1usize << (LOG_ADDRESS_SPACE - addr_rshift(metadata_spec) as usize)
}

#[inline(always)]
pub(crate) fn meta_byte_lshift(metadata_spec: &SideMetadataSpec, data_addr: Address) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits as i32;
    if bits_num_log >= 3 {
        return 0;
    }
    let rem_shift = BITS_IN_WORD as i32 - ((LOG_BITS_IN_BYTE as i32) - bits_num_log);
    ((((data_addr >> metadata_spec.log_bytes_in_region) << rem_shift) >> rem_shift) << bits_num_log)
        as u8
}

#[inline(always)]
pub(crate) fn meta_byte_mask(metadata_spec: &SideMetadataSpec) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits;
    ((1usize << (1usize << bits_num_log)) - 1) as u8
}
