use super::SideMetadataSpec;
use crate::util::{
    constants::{self, LOG_BITS_IN_BYTE},
    heap::layout::vm_layout::{BYTES_IN_CHUNK, CHUNK_MASK, LOG_BYTES_IN_CHUNK},
    memory, Address,
};
use std::io::Result;

use super::constants::{
    LOCAL_SIDE_METADATA_BASE_ADDRESS, LOCAL_SIDE_METADATA_PER_CHUNK,
    LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO,
};
#[cfg(test)]
use super::ensure_munmap_metadata;
use crate::MMAPPER;

pub(super) fn address_to_chunked_meta_address(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let log_bits_num = metadata_spec.log_num_of_bits as i32;
    let log_bytes_in_region = metadata_spec.log_bytes_in_region;

    let rshift = (LOG_BITS_IN_BYTE as i32) - log_bits_num;

    let meta_chunk_addr = address_to_meta_chunk_addr(data_addr);
    let internal_addr = data_addr & CHUNK_MASK;
    let effective_addr = internal_addr >> log_bytes_in_region;
    let second_offset = if rshift >= 0 {
        effective_addr >> rshift
    } else {
        effective_addr << (-rshift)
    };

    meta_chunk_addr + metadata_spec.get_rel_offset() + second_offset
}

/// Returns the size in bytes that gets munmapped.
#[cfg(test)]
pub(crate) fn ensure_munmap_chunked_metadata_space(
    start: Address,
    size: usize,
    spec: &SideMetadataSpec,
) -> usize {
    use super::address_to_meta_address;
    use crate::util::constants::BYTES_IN_PAGE;
    let meta_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
    // per chunk policy-specific metadata for 32-bits targets
    let chunk_num = ((start + size - 1usize).align_down(BYTES_IN_CHUNK)
        - start.align_down(BYTES_IN_CHUNK))
        / BYTES_IN_CHUNK;
    if chunk_num == 0 {
        let size_to_unmap = address_to_meta_address(spec, start + size) - meta_start;
        ensure_munmap_metadata(meta_start, size_to_unmap);

        size_to_unmap
    } else {
        let mut total_unmapped = 0;
        let second_data_chunk = (start + 1usize).align_up(BYTES_IN_CHUNK);
        // unmap the first sub-chunk
        let first_sub_chunk_size = address_to_meta_address(spec, second_data_chunk) - meta_start;
        ensure_munmap_metadata(meta_start, first_sub_chunk_size);
        total_unmapped += first_sub_chunk_size;

        let last_data_chunk = (start + size).align_down(BYTES_IN_CHUNK);
        let last_meta_chunk = address_to_meta_address(spec, last_data_chunk);
        let last_sub_chunk_size = address_to_meta_address(spec, start + size) - last_meta_chunk;
        // unmap the last sub-chunk
        ensure_munmap_metadata(last_meta_chunk, last_sub_chunk_size);
        total_unmapped += last_sub_chunk_size;

        let mut next_data_chunk = second_data_chunk;
        // unmap all chunks in the middle
        while next_data_chunk != last_data_chunk {
            let to_unmap = metadata_bytes_per_chunk(spec.log_bytes_in_region, spec.log_num_of_bits);
            ensure_munmap_metadata(address_to_meta_address(spec, next_data_chunk), to_unmap);
            total_unmapped += to_unmap;
            next_data_chunk += BYTES_IN_CHUNK;
        }

        total_unmapped
    }
}

pub(super) const fn address_to_meta_chunk_addr(data_addr: Address) -> Address {
    LOCAL_SIDE_METADATA_BASE_ADDRESS
        .add((data_addr.as_usize() & !CHUNK_MASK) >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
}

pub(super) const fn metadata_bytes_per_chunk(
    log_bytes_in_region: usize,
    log_num_of_bits: usize,
) -> usize {
    1usize
        << (LOG_BYTES_IN_CHUNK - (constants::LOG_BITS_IN_BYTE as usize) + log_num_of_bits
            - log_bytes_in_region)
}

/// Unmaps the metadata for a single chunk starting at `start`
#[cfg(test)]
pub(crate) fn ensure_munmap_metadata_chunk(start: Address, local_per_chunk: usize) {
    if local_per_chunk != 0 {
        let policy_meta_start = address_to_meta_chunk_addr(start);
        assert!(memory::munmap(policy_meta_start, local_per_chunk).is_ok())
    }
}

/// Returns the size in bytes that gets mmapped in the function if success.
pub(super) fn try_map_per_chunk_metadata_space(
    start: Address,
    size: usize,
    local_per_chunk: usize,
    no_reserve: bool,
) -> Result<usize> {
    let mut aligned_start = start.align_down(BYTES_IN_CHUNK);
    let aligned_end = (start + size).align_up(BYTES_IN_CHUNK);

    // first chunk is special, as it might already be mapped, so it shouldn't be unmapped on failure
    let mut munmap_first_chunk: Option<bool> = None;
    // count the total bytes we mmapped
    let mut total_mapped = 0;

    while aligned_start < aligned_end {
        let res = try_mmap_metadata_chunk(aligned_start, local_per_chunk, no_reserve);
        if res.is_err() {
            if munmap_first_chunk.is_some() {
                let mut munmap_start = if munmap_first_chunk.unwrap() {
                    start.align_down(BYTES_IN_CHUNK)
                } else {
                    start.align_down(BYTES_IN_CHUNK) + BYTES_IN_CHUNK
                };
                // The code that was intended to deal with the failing cases is commented out.
                // See the comment below. Suppress the warning for now.
                #[allow(clippy::never_loop)]
                // Failure: munmap what has been mmapped before
                while munmap_start < aligned_start {
                    // Commented out the following as we do not have unmap in Mmapper.
                    // And we cannot guarantee that the memory to be munmapped does not include any useful data.
                    // However, as we cannot map the address we need for sidemetadata, it is a fatal error
                    // anyway, we do not need to munmap or anything as we cannot recover from it.
                    // ensure_munmap_metadata_chunk(munmap_start, local_per_chunk);
                    munmap_start += LOCAL_SIDE_METADATA_PER_CHUNK;
                    panic!("We have failed mmap");
                }
            }
            trace!(
                "try_map_per_chunk_metadata_space({}, 0x{:x}, 0x{:x}) -> {:#?}",
                start,
                size,
                local_per_chunk,
                res
            );
            return Result::Err(res.err().unwrap());
        }
        if munmap_first_chunk.is_none() {
            // if first chunk is newly mapped, it needs munmap on failure
            munmap_first_chunk = Some(memory::result_is_mapped(res));
        }
        aligned_start += BYTES_IN_CHUNK;
        total_mapped += local_per_chunk;
    }

    trace!(
        "try_map_per_chunk_metadata_space({}, 0x{:x}, 0x{:x}) -> OK(())",
        start,
        size,
        local_per_chunk
    );
    Ok(total_mapped)
}

// Try to map side metadata for the chunk starting at `start`
pub(super) fn try_mmap_metadata_chunk(
    start: Address,
    local_per_chunk: usize,
    no_reserve: bool,
) -> Result<()> {
    debug_assert!(start.is_aligned_to(BYTES_IN_CHUNK));

    let policy_meta_start = address_to_meta_chunk_addr(start);
    let pages = crate::util::conversions::bytes_to_pages_up(local_per_chunk);
    if !no_reserve {
        // We have reserved the memory
        MMAPPER.ensure_mapped(policy_meta_start, pages)
    } else {
        MMAPPER.quarantine_address_range(policy_meta_start, pages)
    }
}
