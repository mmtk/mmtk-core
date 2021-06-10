use super::*;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::memory;
use crate::util::metadata::MetadataSpec;
use crate::util::{constants, Address};
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering};

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
pub fn ensure_metadata_is_mapped(metadata_spec: MetadataSpec, data_addr: Address) {
    let meta_start = address_to_meta_address(metadata_spec, data_addr).align_down(BYTES_IN_PAGE);

    debug!(
        "ensure_metadata_is_mapped({}).meta_start({})",
        data_addr, meta_start
    );

    memory::panic_if_unmapped(meta_start, BYTES_IN_PAGE);
}

#[inline(always)]
pub fn load_atomic(metadata_spec: MetadataSpec, data_addr: Address, order: Ordering) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    let res = if bits_num_log <= 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;
        let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(order) };

        ((byte_val & mask) as usize) >> lshift
    } else if bits_num_log == 4 {
        unsafe { meta_addr.atomic_load::<AtomicU16>(order) as usize }
    } else if bits_num_log == 5 {
        unsafe { meta_addr.atomic_load::<AtomicU32>(order) as usize }
    } else if bits_num_log == 6 {
        unsafe { meta_addr.atomic_load::<AtomicUsize>(order) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    };

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_load(&metadata_spec, data_addr, res);

    res
}

pub fn store_atomic(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    metadata: usize,
    order: Ordering,
) {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_val = (old_val & !mask) | ((metadata as u8) << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_val = (old_val & !mask) | ((metadata as u8) << lshift);
        }
    } else if bits_num_log == 3 {
        unsafe { meta_addr.atomic_store::<AtomicU8>(metadata as u8, order) };
    } else if bits_num_log == 4 {
        unsafe { meta_addr.atomic_store::<AtomicU16>(metadata as u16, order) };
    } else if bits_num_log == 5 {
        unsafe { meta_addr.atomic_store::<AtomicU32>(metadata as u32, order) };
    } else if bits_num_log == 6 {
        unsafe { meta_addr.atomic_store::<AtomicUsize>(metadata as usize, order) };
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    }

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_store(metadata_spec, data_addr, metadata);
}

pub fn compare_exchange_atomic(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    old_metadata: usize,
    new_metadata: usize,
    success_order: Ordering,
    failure_order: Ordering,
) -> bool {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    debug!(
        "compare_exchange_atomic({:?}, {}, {}, {})",
        metadata_spec, data_addr, old_metadata, new_metadata
    );
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    #[allow(clippy::let_and_return)]
    let res = if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(success_order) };
        let expected_old_byte = (real_old_byte & !mask) | ((old_metadata as u8) << lshift);
        let expected_new_byte = (expected_old_byte & !mask) | ((new_metadata as u8) << lshift);

        unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(
                    expected_old_byte,
                    expected_new_byte,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if bits_num_log == 3 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(
                    old_metadata as u8,
                    new_metadata as u8,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if bits_num_log == 4 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU16>(
                    old_metadata as u16,
                    new_metadata as u16,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if bits_num_log == 5 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicU32>(
                    old_metadata as u32,
                    new_metadata as u32,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if bits_num_log == 6 {
        unsafe {
            meta_addr
                .compare_exchange::<AtomicUsize>(
                    old_metadata,
                    new_metadata,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    };

    #[cfg(feature = "extreme_assertions")]
    if res {
        sanity::verify_store(metadata_spec, data_addr, new_metadata);
    }

    res
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_add_atomic(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    val: usize,
    order: Ordering,
) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    #[allow(clippy::let_and_return)]
    let old_val = if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
        let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
            new_val = (old_val & !mask) | (new_sub_val << lshift);
        }

        (old_val & mask) as usize
    } else if bits_num_log == 3 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU8>()).fetch_add(val as u8, order) as usize }
    } else if bits_num_log == 4 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU16>()).fetch_add(val as u16, order) as usize }
    } else if bits_num_log == 5 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU32>()).fetch_add(val as u32, order) as usize }
    } else if bits_num_log == 6 {
        unsafe { (&*meta_addr.to_ptr::<AtomicUsize>()).fetch_add(val, order) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    };

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_add(metadata_spec, data_addr, val, old_val);

    old_val
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_sub_atomic(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    val: usize,
    order: Ordering,
) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    #[allow(clippy::let_and_return)]
    let old_val = if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
        let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
            new_val = (old_val & !mask) | (new_sub_val << lshift);
        }

        (old_val & mask) as usize
    } else if bits_num_log == 3 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU8>()).fetch_sub(val as u8, order) as usize }
    } else if bits_num_log == 4 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU16>()).fetch_sub(val as u16, order) as usize }
    } else if bits_num_log == 5 {
        unsafe { (&*meta_addr.to_ptr::<AtomicU32>()).fetch_sub(val as u32, order) as usize }
    } else if bits_num_log == 6 {
        unsafe { (&*meta_addr.to_ptr::<AtomicUsize>()).fetch_sub(val, order) }
    } else {
        unreachable!(
            "side metadata > {}-bits is not supported!",
            constants::BITS_IN_WORD
        );
    };

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_sub(metadata_spec, data_addr, val, old_val);

    old_val
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
pub unsafe fn load(metadata_spec: MetadataSpec, data_addr: Address) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

    #[allow(clippy::let_and_return)]
    let res = if bits_num_log <= 3 {
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
    };

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_load(&metadata_spec, data_addr, res);

    res
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
pub unsafe fn store(metadata_spec: MetadataSpec, data_addr: Address, metadata: usize) {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.num_of_bits.trailing_zeros();

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

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_store(metadata_spec, data_addr, metadata);
}

/// Bulk-zero a specific metadata for a chunk.
///
/// # Arguments
///
/// * `metadata_spec` - The specification of the target side metadata.
///
/// * `chunk_start` - The starting address of the chunk whose metadata is being zeroed.
///
pub fn bzero_metadata(metadata_spec: MetadataSpec, start: Address, size: usize) {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    debug_assert!(
        start.is_aligned_to(BYTES_IN_PAGE) && meta_byte_lshift(metadata_spec, start) == 0
    );

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_bzero(metadata_spec, start, size);

    let meta_start = address_to_meta_address(metadata_spec, start);
    if cfg!(target_pointer_width = "64") || metadata_spec.is_global {
        memory::zero(
            meta_start,
            address_to_meta_address(metadata_spec, start + size) - meta_start,
        );
    }
    #[cfg(target_pointer_width = "32")]
    if !metadata_spec.is_global {
        use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;

        // per chunk policy-specific metadata for 32-bits targets
        let chunk_num = ((start + size).align_down(BYTES_IN_CHUNK)
            - start.align_down(BYTES_IN_CHUNK))
            / BYTES_IN_CHUNK;
        if chunk_num == 0 {
            memory::zero(
                meta_start,
                address_to_meta_address(metadata_spec, start + size) - meta_start,
            );
        } else {
            let second_data_chunk = start.align_up(BYTES_IN_CHUNK);
            // bzero the first sub-chunk
            memory::zero(
                meta_start,
                address_to_meta_address(metadata_spec, second_data_chunk) - meta_start,
            );
            let last_data_chunk = (start + size).align_down(BYTES_IN_CHUNK);
            let last_meta_chunk = address_to_meta_address(metadata_spec, last_data_chunk);
            // bzero the last sub-chunk
            memory::zero(
                last_meta_chunk,
                address_to_meta_address(metadata_spec, start + size) - last_meta_chunk,
            );
            let mut next_data_chunk = second_data_chunk;
            // bzero all chunks in the middle
            while next_data_chunk != last_data_chunk {
                memory::zero(
                    address_to_meta_address(metadata_spec, next_data_chunk),
                    metadata_bytes_per_chunk(
                        metadata_spec.log_min_obj_size,
                        metadata_spec.num_of_bits,
                    ),
                );
                next_data_chunk += BYTES_IN_CHUNK;
            }
        }
    }
}
