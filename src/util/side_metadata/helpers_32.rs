use super::{constants::*, MappingState};
use crate::util::{
    constants,
    heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK},
    Address,
};

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
        let result = unsafe { libc::munmap(policy_meta_start.to_mut_ptr(), local_per_chunk) };
        assert_eq!(result, 0);
    }
}

pub fn try_map_per_chunk_metadata_space(
    start: Address,
    size: usize,
    local_per_chunk: usize,
) -> bool {
    let mut aligned_start = start.align_down(BYTES_IN_CHUNK);
    let aligned_end = (start + size).align_up(BYTES_IN_CHUNK);

    // first chunk is special, as it might already be mapped, so it shouldn't be unmapped on failure
    let mut munmap_first_chunk: Option<bool> = None;

    while aligned_start < aligned_end {
        let res = try_mmap_metadata_chunk(aligned_start, local_per_chunk);
        if !res.is_mappable() {
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
            return false;
        }
        if munmap_first_chunk.is_none() {
            // if first chunk is newly mapped, it needs munmap on failure
            munmap_first_chunk = Some(res.is_mapped());
        }
        aligned_start += BYTES_IN_CHUNK;
    }

    true
}

// Try to map side metadata for the chunk starting at `start`
pub fn try_mmap_metadata_chunk(start: Address, local_per_chunk: usize) -> MappingState {
    trace!(
        "try_mmap_metadata_chunk({}, 0x{:x})",
        start,
        local_per_chunk
    );

    debug_assert!(start.is_aligned_to(BYTES_IN_CHUNK));

    let policy_meta_start = address_to_meta_chunk_addr(start);

    let prot = libc::PROT_READ | libc::PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;

    if local_per_chunk != 0 {
        let result: *mut libc::c_void = unsafe {
            libc::mmap(
                policy_meta_start.to_mut_ptr(),
                local_per_chunk,
                prot,
                flags,
                -1,
                0,
            )
        };

        if result == libc::MAP_FAILED {
            let err = unsafe { *libc::__errno_location() };
            if err == libc::EEXIST {
                return MappingState::WasMapped;
            } else {
                return MappingState::NotMappable;
            }
        }
    }

    MappingState::IsMapped
}
