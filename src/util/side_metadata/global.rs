use super::constants::*;
use super::helpers::*;
use crate::util::{constants, Address};
use crate::util::{heap::layout::vm_layout_constants::BYTES_IN_CHUNK, memory};
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering};

#[derive(Clone, Copy)]
pub enum SideMetadataScope {
    Global,
    PolicySpecific,
}

impl SideMetadataScope {
    pub fn is_global(&self) -> bool {
        matches!(self, SideMetadataScope::Global)
    }
}

/// This struct stores the specification of a side metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy)]
pub struct SideMetadataSpec {
    pub scope: SideMetadataScope,
    pub offset: usize,
    pub log_num_of_bits: usize,
    pub log_min_obj_size: usize,
}

/// Represents the mapping state of a metadata page.
///
/// `NotMappable` indicates whether the page is mappable by MMTK.
/// `IsMapped` indicates that the page is newly mapped by MMTK, and `WasMapped` means the page was already mapped.
pub enum MappingState {
    NotMappable,
    IsMapped,
    WasMapped,
}

impl MappingState {
    pub fn is_mapped(&self) -> bool {
        matches!(self, MappingState::IsMapped)
    }

    pub fn is_mappable(&self) -> bool {
        !matches!(self, MappingState::NotMappable)
    }

    pub fn was_mapped(&self) -> bool {
        matches!(self, MappingState::WasMapped)
    }
}

// ** NOTE: **
//  Regardless of the number of bits in a metadata unit, we always represent its content as a word.

/// Tries to map the required metadata space and returns `true` is successful.
///
/// # Arguments
///
/// * `start` - The starting address of the source data.
///
/// * `size` - The size of the source data (in bytes).
///
/// * `global_per_chunk` - The number of bytes of global side metadata required per chunk.
///
/// * `local_per_chunk` - The number of bytes of policy-specific side metadata required per chunk.
///
pub fn try_map_metadata_space(
    start: Address,
    size: usize,
    global_per_chunk: usize,
    local_per_chunk: usize,
) -> bool {
    let mut aligned_start = start.align_down(BYTES_IN_CHUNK);
    let aligned_end = (start + size).align_up(BYTES_IN_CHUNK);

    // first chunk is special, as it might already be mapped, so it shouldn't be unmapped on failure
    let mut munmap_first_chunk: Option<bool> = None;

    while aligned_start < aligned_end {
        let res = try_mmap_metadata_chunk(aligned_start, global_per_chunk, local_per_chunk);
        if !res.is_mappable() {
            if munmap_first_chunk.is_some() {
                let mut munmap_start = if munmap_first_chunk.unwrap() {
                    start.align_down(BYTES_IN_CHUNK)
                } else {
                    start.align_down(BYTES_IN_CHUNK) + BYTES_IN_CHUNK
                };
                // Failure: munmap what has been mmapped before
                while munmap_start < aligned_start {
                    ensure_munmap_metadata_chunk(munmap_start, global_per_chunk, local_per_chunk);
                    munmap_start += SIDE_METADATA_PER_CHUNK;
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
pub fn try_mmap_metadata_chunk(
    start: Address,
    global_per_chunk: usize,
    local_per_chunk: usize,
) -> MappingState {
    trace!(
        "try_mmap_metadata_chunk({}, 0x{:x}, 0x{:x})",
        start,
        global_per_chunk,
        local_per_chunk
    );
    let global_meta_start = address_to_meta_chunk_addr(start);

    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;

    if global_per_chunk != 0 {
        let result: *mut libc::c_void = unsafe {
            libc::mmap(
                global_meta_start.to_mut_ptr(),
                global_per_chunk,
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

    let policy_meta_start = global_meta_start + POLICY_SIDE_METADATA_OFFSET;

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

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
pub fn ensure_metadata_chunk_is_mmaped(metadata_spec: SideMetadataSpec, data_addr: Address) {
    let meta_start = if metadata_spec.scope.is_global() {
        address_to_meta_chunk_addr(data_addr)
    } else {
        address_to_meta_chunk_addr(data_addr) + POLICY_SIDE_METADATA_OFFSET
    };

    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;

    let result: *mut libc::c_void = unsafe {
        libc::mmap(
            meta_start.to_mut_ptr(),
            constants::BYTES_IN_PAGE,
            prot,
            flags,
            -1,
            0,
        )
    };

    assert!(
        result == libc::MAP_FAILED && unsafe { *libc::__errno_location() } == libc::EEXIST,
        "Metadata space is not mapped for data_addr({})",
        data_addr
    );
}

/// Unmaps the metadata for a single chunk starting at `start`
pub fn ensure_munmap_metadata_chunk(
    start: Address,
    global_per_chunk: usize,
    local_per_chunk: usize,
) {
    let global_meta_start = address_to_meta_chunk_addr(start);
    let result = unsafe { libc::munmap(global_meta_start.to_mut_ptr(), global_per_chunk) };
    assert_eq!(result, 0);

    let policy_meta_start = global_meta_start + POLICY_SIDE_METADATA_OFFSET;
    let result = unsafe { libc::munmap(policy_meta_start.to_mut_ptr(), local_per_chunk) };
    assert_eq!(result, 0);
}

#[inline(always)]
pub fn load_atomic(metadata_spec: SideMetadataSpec, data_addr: Address) -> usize {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log <= 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;
        let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(Ordering::SeqCst) };

        ((byte_val & mask) as usize) >> lshift
    } else if bits_num_log == 4 {
        unsafe { meta_addr.atomic_load::<AtomicU16>(Ordering::SeqCst) as usize }
    } else if bits_num_log == 5 {
        unsafe { meta_addr.atomic_load::<AtomicU32>(Ordering::SeqCst) as usize }
    } else if bits_num_log == 6 {
        unsafe { meta_addr.atomic_load::<AtomicUsize>(Ordering::SeqCst) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

pub fn store_atomic(metadata_spec: SideMetadataSpec, data_addr: Address, metadata: usize) {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_val = (old_val & !mask) | ((metadata as u8) << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_val = (old_val & !mask) | ((metadata as u8) << lshift);
        }
    } else if bits_num_log == 3 {
        unsafe { meta_addr.atomic_store::<AtomicU8>(metadata as u8, Ordering::SeqCst) };
    } else if bits_num_log == 4 {
        unsafe { meta_addr.atomic_store::<AtomicU16>(metadata as u16, Ordering::SeqCst) };
    } else if bits_num_log == 5 {
        unsafe { meta_addr.atomic_store::<AtomicU32>(metadata as u32, Ordering::SeqCst) };
    } else if bits_num_log == 6 {
        unsafe { meta_addr.atomic_store::<AtomicUsize>(metadata as usize, Ordering::SeqCst) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

pub fn compare_exchange_atomic(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    old_metadata: usize,
    new_metadata: usize,
) -> bool {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(Ordering::SeqCst) };
        let expected_old_byte = (real_old_byte & !mask) | ((old_metadata as u8) << lshift);
        let expected_new_byte = expected_old_byte | ((new_metadata as u8) << lshift);

        unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(
                    expected_old_byte,
                    expected_new_byte,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        }
    } else if bits_num_log == 3 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(
                    old_metadata as u8,
                    new_metadata as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        }
    } else if bits_num_log == 4 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU16>(
                    old_metadata as u16,
                    new_metadata as u16,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        }
    } else if bits_num_log == 5 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU32>(
                    old_metadata as u32,
                    new_metadata as u32,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        }
    } else if bits_num_log == 6 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicUsize>(
                    old_metadata,
                    new_metadata,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_add_atomic(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> usize {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
        let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
            new_val = (old_val & !mask) | (new_sub_val << lshift);
        }

        (old_val & mask) as usize
    } else if bits_num_log == 3 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU8>()).fetch_add(val as u8, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 4 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU16>()).fetch_add(val as u16, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 5 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU32>()).fetch_add(val as u32, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 6 {
        unsafe { (&*meta_addr.to_ptr::<AtomicUsize>()).fetch_add(val, Ordering::SeqCst) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_sub_atomic(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> usize {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
        let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
            new_val = (old_val & !mask) | (new_sub_val << lshift);
        }

        (old_val & mask) as usize
    } else if bits_num_log == 3 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU8>()).fetch_sub(val as u8, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 4 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU16>()).fetch_sub(val as u16, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 5 {
        unsafe {
            (&*meta_addr.to_ptr::<AtomicU32>()).fetch_sub(val as u32, Ordering::SeqCst) as usize
        }
    } else if bits_num_log == 6 {
        unsafe { (&*meta_addr.to_ptr::<AtomicUsize>()).fetch_sub(val, Ordering::SeqCst) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

/// Non-atomic load of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
pub unsafe fn load(metadata_spec: SideMetadataSpec, data_addr: Address) -> usize {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log <= 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;
        let byte_val = meta_addr.load::<u8>();

        ((byte_val & mask) as usize) >> lshift
    } else if bits_num_log == 4 {
        meta_addr.load::<u16>() as usize
    } else if bits_num_log == 5 {
        meta_addr.load::<u32>() as usize
    } else if bits_num_log == 6 {
        meta_addr.load::<usize>() as usize
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

/// Non-atomic store of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
pub unsafe fn store(metadata_spec: SideMetadataSpec, data_addr: Address, metadata: usize) {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_chunk_is_mmaped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let old_val = meta_addr.load::<u8>();
        let new_val = (old_val & !mask) | ((metadata as u8) << lshift);

        meta_addr.store::<u8>(new_val);
    } else if bits_num_log == 3 {
        meta_addr.store::<u8>(metadata as u8);
    } else if bits_num_log == 4 {
        meta_addr.store::<u16>(metadata as u16);
    } else if bits_num_log == 5 {
        meta_addr.store::<u32>(metadata as u32);
    } else if bits_num_log == 6 {
        meta_addr.store::<usize>(metadata as usize);
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }
}

/// Bulk-zero a specific metadata for a chunk.
///
/// # Arguments
///
/// * `metadata_spec` - The specification of the target side metadata.
///
/// * `chunk_start` - The starting address of the chunk whose metadata is being zeroed.
///
pub fn bzero_metadata_for_chunk(metadata_spec: SideMetadataSpec, chunk_start: Address) {
    debug_assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));

    let meta_start = address_to_meta_address(metadata_spec, chunk_start);
    let meta_size = meta_bytes_per_chunk(
        metadata_spec.log_min_obj_size,
        metadata_spec.log_num_of_bits,
    );
    memory::zero(meta_start, meta_size);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::side_metadata::helpers;
    use crate::util::{constants, Address};

    #[test]
    fn test_side_metadata_try_mmap_metadata_chunk() {
        let gspec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };
        let lspec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: 0,
            log_num_of_bits: 1,
            log_min_obj_size: 0,
        };

        assert!(try_map_metadata_space(
            unsafe { Address::from_usize(0) },
            1,
            helpers::meta_bytes_per_chunk(0, 0),
            helpers::meta_bytes_per_chunk(0, 1)
        ));

        ensure_metadata_chunk_is_mmaped(gspec, unsafe { Address::from_usize(0) });
        ensure_metadata_chunk_is_mmaped(lspec, unsafe { Address::from_usize(0) });
        ensure_metadata_chunk_is_mmaped(gspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK - 1)
        });
        ensure_metadata_chunk_is_mmaped(lspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK - 1)
        });

        assert!(try_map_metadata_space(
            unsafe { Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK) },
            vm_layout_constants::BYTES_IN_CHUNK + 1,
            helpers::meta_bytes_per_chunk(3, 2),
            helpers::meta_bytes_per_chunk(4, 2)
        ));

        ensure_metadata_chunk_is_mmaped(gspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK)
        });
        ensure_metadata_chunk_is_mmaped(lspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK)
        });
        ensure_metadata_chunk_is_mmaped(gspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK * 3 - 1)
        });
        ensure_metadata_chunk_is_mmaped(lspec, unsafe {
            Address::from_usize(vm_layout_constants::BYTES_IN_CHUNK * 3 - 1)
        });
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_ge8bits() {
        let data_addr = vm_layout_constants::HEAP_START;

        let metadata_1_spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 4,
            log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
        };

        let metadata_2_spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: helpers::meta_bytes_per_chunk(
                metadata_1_spec.log_min_obj_size,
                metadata_1_spec.log_num_of_bits,
            ),
            log_num_of_bits: 3,
            log_min_obj_size: 7,
        };
        assert!(try_map_metadata_space(
            data_addr,
            constants::BYTES_IN_PAGE,
            helpers::meta_bytes_per_chunk(
                metadata_2_spec.log_min_obj_size,
                metadata_2_spec.log_num_of_bits
            ) + helpers::meta_bytes_per_chunk(
                metadata_1_spec.log_min_obj_size,
                metadata_1_spec.log_num_of_bits
            ),
            0
        ));

        let zero = fetch_add_atomic(metadata_1_spec, data_addr, 5);
        assert_eq!(zero, 0);

        let five = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(five, 5);

        let zero = fetch_add_atomic(metadata_2_spec, data_addr, 5);
        assert_eq!(zero, 0);

        let five = load_atomic(metadata_2_spec, data_addr);
        assert_eq!(five, 5);

        let another_five = fetch_sub_atomic(metadata_1_spec, data_addr, 2);
        assert_eq!(another_five, 5);

        let three = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(three, 3);

        let another_five = fetch_sub_atomic(metadata_2_spec, data_addr, 2);
        assert_eq!(another_five, 5);

        let three = load_atomic(metadata_2_spec, data_addr);
        assert_eq!(three, 3);
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_2bits() {
        let data_addr =
            vm_layout_constants::HEAP_START + (vm_layout_constants::BYTES_IN_CHUNK << 1);

        let metadata_1_spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 1,
            log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
        };

        assert!(try_map_metadata_space(
            data_addr,
            constants::BYTES_IN_PAGE,
            helpers::meta_bytes_per_chunk(
                metadata_1_spec.log_min_obj_size,
                metadata_1_spec.log_num_of_bits
            ),
            0
        ));

        let zero = fetch_add_atomic(metadata_1_spec, data_addr, 2);
        assert_eq!(zero, 0);

        let two = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(two, 2);

        let another_two = fetch_sub_atomic(metadata_1_spec, data_addr, 1);
        assert_eq!(another_two, 2);

        let one = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(one, 1);
    }

    #[test]
    fn test_side_metadata_bzero_metadata_for_chunk() {
        let data_addr =
            vm_layout_constants::HEAP_START + (vm_layout_constants::BYTES_IN_CHUNK << 2);

        let metadata_1_spec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: 0,
            log_num_of_bits: 4,
            log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
        };

        let metadata_2_spec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: helpers::meta_bytes_per_chunk(
                metadata_1_spec.log_min_obj_size,
                metadata_1_spec.log_num_of_bits,
            ),
            log_num_of_bits: 3,
            log_min_obj_size: 7,
        };
        assert!(try_map_metadata_space(
            data_addr,
            constants::BYTES_IN_PAGE,
            0,
            helpers::meta_bytes_per_chunk(
                metadata_2_spec.log_min_obj_size,
                metadata_2_spec.log_num_of_bits
            ) + helpers::meta_bytes_per_chunk(
                metadata_1_spec.log_min_obj_size,
                metadata_1_spec.log_num_of_bits
            )
        ));

        let zero = fetch_add_atomic(metadata_1_spec, data_addr, 5);
        assert_eq!(zero, 0);

        let five = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(five, 5);

        let zero = fetch_add_atomic(metadata_2_spec, data_addr, 5);
        assert_eq!(zero, 0);

        let five = load_atomic(metadata_2_spec, data_addr);
        assert_eq!(five, 5);

        bzero_metadata_for_chunk(metadata_2_spec, data_addr);

        let five = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(five, 5);
        let five = load_atomic(metadata_2_spec, data_addr);
        assert_eq!(five, 0);

        bzero_metadata_for_chunk(metadata_1_spec, data_addr);

        let five = load_atomic(metadata_1_spec, data_addr);
        assert_eq!(five, 0);
        let five = load_atomic(metadata_2_spec, data_addr);
        assert_eq!(five, 0);
    }
}
