use super::*;
use crate::util::memory;
use crate::util::Address;
use crate::util::{
    constants::{BITS_IN_WORD, BYTES_IN_PAGE, LOG_BITS_IN_BYTE},
    heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE,
};
use std::io::Result;
use super::constants::*;


pub(crate) const fn address_to_meta_chunk_addr(data_addr: Address) -> Address {

    LOCAL_SIDE_METADATA_BASE_ADDRESS
        + ((data_addr.as_usize() & !CHUNK_MASK) >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
}

/// Performs address translation in contiguous metadata spaces (e.g. global and policy-specific in 64-bits, and global in 32-bits)
#[inline(always)]
pub(crate) fn address_to_contiguous_meta_address(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let log_bits_num = metadata_spec.log_num_of_bits as i32;
    let log_min_obj_size = metadata_spec.log_min_obj_size as usize;

    let rshift = (LOG_BITS_IN_BYTE as i32) - log_bits_num;

    unsafe {
        if rshift >= 0 {
            Address::from_usize(metadata_spec.offset + ((data_addr >> log_min_obj_size) >> rshift))
        } else {
            Address::from_usize(
                metadata_spec.offset + ((data_addr >> log_min_obj_size) << (-rshift)),
            )
        }
    }
}

/// Unmaps the specified metadata range, or panics.
pub(super) fn ensure_munmap_metadata(start: Address, size: usize) {
    trace!("ensure_munmap_metadata({}, 0x{:x})", start, size);

    assert!(memory::try_munmap(start, size).is_ok())
}

/// Unmaps a metadata space (`spec`) for the specified data address range (`start` and `size`)
pub(super) fn ensure_munmap_contiguos_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
) {
    // nearest page-aligned starting address
    let mmap_start = address_to_meta_address(*spec, start).align_down(BYTES_IN_PAGE);
    // nearest page-aligned ending address
    let mmap_size =
        address_to_meta_address(*spec, start + size).align_up(BYTES_IN_PAGE) - mmap_start;
    if mmap_size > 0 {
        ensure_munmap_metadata(mmap_start, mmap_size);
    }
}

/// Tries to mmap the metadata space (`spec`) for the specified data address range (`start` and `size`).
/// Setting `no_reserve` to true means the function will only map address range, without reserving swap-space/physical memory.
pub(super) fn try_mmap_contiguous_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
    no_reserve: bool,
) -> Result<()> {
    debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
    debug_assert!(size % BYTES_IN_PAGE == 0);

    // nearest page-aligned starting address
    let mmap_start = address_to_meta_address(*spec, start).align_down(BYTES_IN_PAGE);
    // nearest page-aligned ending address
    let mmap_size =
        address_to_meta_address(*spec, start + size).align_up(BYTES_IN_PAGE) - mmap_start;
    if mmap_size > 0 {
        // FIXME - This assumes that we never mmap a metadata page twice.
        // While this never happens in our current use-cases where the minimum data mmap size is a chunk and the metadata ratio is larger than 1/64, it could happen if (min_data_mmap_size * metadata_ratio) is smaller than a page.
        // E.g. the current implementation detects such a case as an overlap and returns false.
        if !no_reserve {
            try_mmap_metadata(mmap_start, mmap_size)
        } else {
            try_mmap_metadata_address_range(mmap_start, mmap_size)
        }
    } else {
        Ok(())
    }
}

/// Tries to map the specified metadata address range (`start` and `size`), without reserving swap-space for it.
pub(super) fn try_mmap_metadata_address_range(start: Address, size: usize) -> Result<()> {
    let res = memory::mmap_noreserve(start, size);
    trace!(
        "try_mmap_metadata_address_range({}, 0x{:x}) -> {:#?}",
        start,
        size,
        res
    );
    res
}

/// Tries to map the specified metadata space (`start` and `size`), including reservation of swap-space/physical memory.
pub(super) fn try_mmap_metadata(start: Address, size: usize) -> Result<()> {
    debug_assert!(size > 0 && size % BYTES_IN_PAGE == 0);

    let res = memory::dzmmap(start, size);
    trace!("try_mmap_metadata({}, 0x{:x}) -> {:#?}", start, size, res);
    res
}

/// Performs the translation of data address (`data_addr`) to metadata address for the specified metadata (`metadata_spec`).
#[inline(always)]
pub(crate) fn address_to_meta_address(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
) -> Address {
    #[cfg(target_pointer_width = "32")]
    let res = {
        if metadata_spec.scope.is_global() {
            address_to_contiguous_meta_address(metadata_spec, data_addr)
        } else {
            address_to_chunked_meta_address(metadata_spec, data_addr)
        }
    };
    #[cfg(target_pointer_width = "64")]
    let res = { address_to_contiguous_meta_address(metadata_spec, data_addr) };

    trace!(
        "address_to_meta_address(addr: {}, off: 0x{:x}, lbits: {}, lmin: {}) -> 0x{:x}",
        data_addr,
        metadata_spec.offset,
        metadata_spec.log_num_of_bits,
        metadata_spec.log_min_obj_size,
        res
    );

    res
}

const fn addr_rshift(metadata_spec: SideMetadataSpec) -> i32 {
    ((LOG_BITS_IN_BYTE as usize) + metadata_spec.log_min_obj_size - metadata_spec.log_num_of_bits)
        as i32
}

#[allow(dead_code)]
#[inline(always)]
pub(crate) const fn metadata_address_range_size(metadata_spec: SideMetadataSpec) -> usize {
    1usize << (LOG_ADDRESS_SPACE - addr_rshift(metadata_spec) as usize)
}

#[inline(always)]
pub(crate) fn meta_byte_lshift(metadata_spec: SideMetadataSpec, data_addr: Address) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits as i32;
    if bits_num_log >= 3 {
        return 0;
    }
    let rem_shift = BITS_IN_WORD as i32 - ((LOG_BITS_IN_BYTE as i32) - bits_num_log);
    ((((data_addr >> metadata_spec.log_min_obj_size) << rem_shift) >> rem_shift) << bits_num_log)
        as u8
}

#[inline(always)]
pub(crate) fn meta_byte_mask(metadata_spec: SideMetadataSpec) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits;
    ((1usize << (1usize << bits_num_log)) - 1) as u8
}
