use super::helpers::*;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::memory;
use crate::util::{constants, Address};
use std::sync::{
    atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Copy)]
pub enum SideMetadataScope {
    Global,
    PolicySpecific,
}

impl SideMetadataScope {
    pub const fn is_global(&self) -> bool {
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
    global_metadata_spec_vec: Arc<Vec<SideMetadataSpec>>,
    local_metadata_spec_vec: Arc<Vec<SideMetadataSpec>>,
) -> bool {
    debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
    debug_assert!(size % BYTES_IN_PAGE == 0);

    for i in 0..global_metadata_spec_vec.len() {
        let spec = global_metadata_spec_vec[i];
        // nearest page-aligned starting address
        let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
        // nearest page-aligned ending address
        let mmap_size = address_to_meta_address(spec, start + size)
            .align_up(BYTES_IN_PAGE)
            .as_usize()
            - mmap_start.as_usize();
        if mmap_size > 0 {
            // FIXME - This assumes that we never mmap a metadata page twice.
            // While this never happens in our current use-cases where the minimum data mmap size is a chunk and the metadata ratio is larger than 1/64, it could happen if (min_data_mmap_size * metadata_ratio) is smaller than a page.
            // E.g. the current implementation detects such a case as an overlap and returns false.
            if !try_mmap_metadata(mmap_start, mmap_size).is_mapped() {
                return false;
            }
        }
    }

    for i in 0..local_metadata_spec_vec.len() {
        let spec = local_metadata_spec_vec[i];
        // nearest page-aligned starting address
        let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
        // nearest page-aligned ending address
        let mmap_size = address_to_meta_address(spec, start + size)
            .align_up(BYTES_IN_PAGE)
            .as_usize()
            - mmap_start.as_usize();
        if mmap_size > 0 && !try_mmap_metadata(mmap_start, mmap_size).is_mapped() {
            return false;
        }
    }

    true
}

// Try to map side metadata for the chunk starting at `start`
fn try_mmap_metadata(start: Address, size: usize) -> MappingState {
    trace!("try_mmap_metadata({}, 0x{:x})", start, size);

    debug_assert!(size > 0 && size % BYTES_IN_PAGE == 0);

    let prot = libc::PROT_READ | libc::PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;

    let result: *mut libc::c_void =
        unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) };

    if result == libc::MAP_FAILED {
        let err = unsafe { *libc::__errno_location() };
        if err == libc::EEXIST {
            return MappingState::WasMapped;
        } else {
            return MappingState::NotMappable;
        }
    }

    MappingState::IsMapped
}

pub fn ensure_unmap_metadata_space(
    start: Address,
    size: usize,
    global_metadata_spec_vec: Arc<Vec<SideMetadataSpec>>,
    local_metadata_spec_vec: Arc<Vec<SideMetadataSpec>>,
) {
    debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
    debug_assert!(size % BYTES_IN_PAGE == 0);

    for i in 0..global_metadata_spec_vec.len() {
        let spec = global_metadata_spec_vec[i];
        // nearest page-aligned starting address
        let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
        // nearest page-aligned ending address
        let mmap_size = address_to_meta_address(spec, start + size)
            .align_up(BYTES_IN_PAGE)
            .as_usize()
            - mmap_start.as_usize();
        if mmap_size > 0 {
            ensure_munmap_metadata(mmap_start, mmap_size);
        }
    }

    for i in 0..local_metadata_spec_vec.len() {
        let spec = local_metadata_spec_vec[i];
        // nearest page-aligned starting address
        let mmap_start = address_to_meta_address(spec, start).align_down(BYTES_IN_PAGE);
        // nearest page-aligned ending address
        let mmap_size = address_to_meta_address(spec, start + size)
            .align_up(BYTES_IN_PAGE)
            .as_usize()
            - mmap_start.as_usize();
        if mmap_size > 0 {
            ensure_munmap_metadata(mmap_start, mmap_size);
        }
    }
}

fn ensure_munmap_metadata(start: Address, size: usize) {
    debug!("try_munmap_metadata({}, 0x{:x})", start, size);

    debug_assert!(size > 0 && size % BYTES_IN_PAGE == 0);
    assert_eq!(unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0);
}

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
pub fn ensure_metadata_is_mapped(metadata_spec: SideMetadataSpec, data_addr: Address) {
    let meta_start = address_to_meta_address(metadata_spec, data_addr).align_down(BYTES_IN_PAGE);

    trace!(
        "ensure_metadata_is_mapped({}).meta_start({})",
        data_addr,
        meta_start
    );

    let prot = libc::PROT_READ | libc::PROT_WRITE;
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

#[inline(always)]
pub fn load_atomic(metadata_spec: SideMetadataSpec, data_addr: Address) -> usize {
    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
        ensure_metadata_is_mapped(metadata_spec, data_addr);
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
pub fn bzero_metadata(metadata_spec: SideMetadataSpec, start: Address, size: usize) {
    debug_assert!(
        start.is_aligned_to(BYTES_IN_PAGE) && meta_byte_lshift(metadata_spec, start) == 0
    );

    let meta_start = address_to_meta_address(metadata_spec, start);
    memory::zero(
        meta_start,
        address_to_meta_address(metadata_spec, start + size) - meta_start,
    );
}
