use crate::util::{
    constants::{self, BYTES_IN_PAGE, LOG_BITS_IN_BYTE},
    heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK},
    memory, Address,
};
use std::io::Result;

use super::{
    address_to_meta_address, ensure_munmap_metadata, SideMetadataSpec, CHUNK_MASK,
    LOCAL_SIDE_METADATA_BASE_ADDRESS, LOCAL_SIDE_METADATA_PER_CHUNK,
    LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO,
};

#[inline(always)]
pub(super) fn address_to_chunked_meta_address(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let log_bits_num = metadata_spec.log_num_of_bits as i32;
    let log_min_obj_size = metadata_spec.log_min_obj_size as usize;

    let rshift = (LOG_BITS_IN_BYTE as i32) - log_bits_num;

    let meta_chunk_addr = address_to_meta_chunk_addr(data_addr);
    let internal_addr = data_addr & CHUNK_MASK;
    let effective_addr = internal_addr >> log_min_obj_size;
    let second_offset = if rshift >= 0 {
        effective_addr >> rshift
    } else {
        effective_addr << (-rshift)
    };

    meta_chunk_addr + metadata_spec.offset + second_offset
}

pub(super) fn ensure_munmap_chunked_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
) {
    let meta_start = address_to_meta_address(*spec, start).align_down(BYTES_IN_PAGE);
    // per chunk policy-specific metadata for 32-bits targets
    let chunk_num = ((start + size - 1usize).align_down(BYTES_IN_CHUNK)
        - start.align_down(BYTES_IN_CHUNK))
        / BYTES_IN_CHUNK;
    if chunk_num == 0 {
        ensure_munmap_metadata(
            meta_start,
            address_to_meta_address(*spec, start + size) - meta_start,
        );
    } else {
        let second_data_chunk = (start + 1usize).align_up(BYTES_IN_CHUNK);
        // unmap the first sub-chunk
        ensure_munmap_metadata(
            meta_start,
            address_to_meta_address(*spec, second_data_chunk) - meta_start,
        );
        let last_data_chunk = (start + size).align_down(BYTES_IN_CHUNK);
        let last_meta_chunk = address_to_meta_address(*spec, last_data_chunk);
        // unmap the last sub-chunk
        ensure_munmap_metadata(
            last_meta_chunk,
            address_to_meta_address(*spec, start + size) - last_meta_chunk,
        );
        let mut next_data_chunk = second_data_chunk;
        // unmap all chunks in the middle
        while next_data_chunk != last_data_chunk {
            ensure_munmap_metadata(
                address_to_meta_address(*spec, next_data_chunk),
                meta_bytes_per_chunk(spec.log_min_obj_size, spec.log_num_of_bits),
            );
            next_data_chunk += BYTES_IN_CHUNK;
        }
    }
}

#[inline(always)]
pub(crate) fn address_to_meta_chunk_addr(data_addr: Address) -> Address {
    LOCAL_SIDE_METADATA_BASE_ADDRESS
        + ((data_addr.as_usize() & !CHUNK_MASK) >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
}

#[inline(always)]
pub(crate) const fn meta_bytes_per_chunk(log_min_obj_size: usize, log_num_of_bits: usize) -> usize {
    1usize
        << (LOG_BYTES_IN_CHUNK - (constants::LOG_BITS_IN_BYTE as usize) - log_min_obj_size
            + log_num_of_bits)
}

/// Unmaps the metadata for a single chunk starting at `start`
pub fn ensure_munmap_metadata_chunk(start: Address, local_per_chunk: usize) {
    if local_per_chunk != 0 {
        let policy_meta_start = address_to_meta_chunk_addr(start);
        assert!(memory::try_munmap(policy_meta_start, local_per_chunk).is_ok())
    }
}

pub fn try_map_per_chunk_metadata_space(
    start: Address,
    size: usize,
    local_per_chunk: usize,
) -> Result<()> {
    let mut aligned_start = start.align_down(BYTES_IN_CHUNK);
    let aligned_end = (start + size).align_up(BYTES_IN_CHUNK);

    // first chunk is special, as it might already be mapped, so it shouldn't be unmapped on failure
    let mut munmap_first_chunk: Option<bool> = None;

    while aligned_start < aligned_end {
        let res = try_mmap_metadata_chunk(aligned_start, local_per_chunk);
        if res.is_err() {
            if munmap_first_chunk.is_some() {
                let mut munmap_start = if munmap_first_chunk.unwrap() {
                    start.align_down(BYTES_IN_CHUNK)
                } else {
                    start.align_down(BYTES_IN_CHUNK) + BYTES_IN_CHUNK
                };
                // Failure: munmap what has been mmapped before
                while munmap_start < aligned_start {
                    ensure_munmap_metadata_chunk(munmap_start, local_per_chunk);
                    munmap_start += LOCAL_SIDE_METADATA_PER_CHUNK;
                }
            }
            trace!(
                "try_map_per_chunk_metadata_space({}, 0x{:x}, 0x{:x}) -> {:#?}",
                start,
                size,
                local_per_chunk,
                res
            );
            return res;
        }
        if munmap_first_chunk.is_none() {
            // if first chunk is newly mapped, it needs munmap on failure
            munmap_first_chunk = Some(memory::result_is_mapped(res));
        }
        aligned_start += BYTES_IN_CHUNK;
    }

    trace!(
        "try_map_per_chunk_metadata_space({}, 0x{:x}, 0x{:x}) -> OK(())",
        start,
        size,
        local_per_chunk
    );
    Ok(())
}

// Try to map side metadata for the chunk starting at `start`
pub fn try_mmap_metadata_chunk(start: Address, local_per_chunk: usize) -> Result<()> {
    debug_assert!(start.is_aligned_to(BYTES_IN_CHUNK));

    let policy_meta_start = address_to_meta_chunk_addr(start);
    memory::dzmmap_noreplace(policy_meta_start, local_per_chunk)
}
