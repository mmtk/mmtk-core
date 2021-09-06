use super::*;
#[cfg(feature = "global_alloc_bit")]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::constants::{BYTES_IN_PAGE, LOG_BITS_IN_BYTE};
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::memory;
use crate::util::{constants, Address};
use std::fmt;
use std::io::Result;
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering};

/// This struct stores the specification of a side metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SideMetadataSpec {
    pub is_global: bool,
    pub offset: SideMetadataOffset,
    /// Number of bits needed per region. E.g. 0 = 1 bit, 1 = 2 bit.
    pub log_num_of_bits: usize,
    /// Number of bytes of the region. E.g. 3 = 8 bytes, 12 = 4096 bytes (page).
    pub log_min_obj_size: usize,
}

impl SideMetadataSpec {
    /// Is offset for this spec Address? (contiguous side metadata for 64 bits, and global specs in 32 bits)
    #[inline(always)]
    pub const fn is_absolute_offset(&self) -> bool {
        self.is_global || cfg!(target_pointer_width = "64")
    }
    /// If offset for this spec relative? (chunked side metadata for local specs in 32 bits)
    #[inline(always)]
    pub const fn is_rel_offset(&self) -> bool {
        !self.is_absolute_offset()
    }

    #[inline(always)]
    pub const fn get_absolute_offset(&self) -> Address {
        debug_assert!(self.is_absolute_offset());
        unsafe { self.offset.addr }
    }

    #[inline(always)]
    pub const fn get_rel_offset(&self) -> usize {
        debug_assert!(self.is_rel_offset());
        unsafe { self.offset.rel_offset }
    }
}

impl fmt::Debug for SideMetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "SideMetadataSpec {{ \
            **is_global: {:?} \
            **offset: {} \
            **log_num_of_bits: 0x{:x} \
            **log_min_obj_size: 0x{:x} \
            }}",
            self.is_global,
            unsafe {
                if self.is_absolute_offset() {
                    format!("0x{:x}", self.offset.addr)
                } else {
                    format!("0x{:x}", self.offset.rel_offset)
                }
            },
            self.log_num_of_bits,
            self.log_min_obj_size
        ))
    }
}

/// A union of Address or relative offset (usize) used to store offset for a side metadata spec.
/// If a spec is contiguous side metadata, it uses address. Othrewise it uses usize.
// The fields are made private on purpose. They can only be accessed from SideMetadata which knows whether it is Address or usize.
#[derive(Clone, Copy)]
pub union SideMetadataOffset {
    addr: Address,
    rel_offset: usize,
}

impl SideMetadataOffset {
    // Get an offset for a fixed address. This is usually used to set offset for the first spec (subsequent ones can be laid out with `layout_after`).
    pub const fn addr(addr: Address) -> Self {
        SideMetadataOffset { addr }
    }

    // Get an offset for a relative offset (usize). This is usually used to set offset for the first spec (subsequent ones can be laid out with `layout_after`).
    pub const fn rel(rel_offset: usize) -> Self {
        SideMetadataOffset { rel_offset }
    }

    /// Get an offset after a spec. This is used to layout another spec immediately after this one.
    #[cfg(target_pointer_width = "64")]
    pub const fn layout_after(spec: &SideMetadataSpec) -> SideMetadataOffset {
        debug_assert!(spec.is_absolute_offset());
        SideMetadataOffset {
            addr: unsafe { spec.offset.addr }
                .add(crate::util::metadata::side_metadata::metadata_address_range_size(spec)),
        }
    }
    /// Get an offset after a spec. This is used to layout another spec immediately after this one.
    #[cfg(target_pointer_width = "32")]
    pub const fn layout_after(spec: &SideMetadataSpec) -> SideMetadataOffset {
        if spec.is_absolute_offset() {
            SideMetadataOffset {
                addr: unsafe { spec.offset.addr }
                    .add(crate::util::metadata::side_metadata::metadata_address_range_size(spec)),
            }
        } else {
            SideMetadataOffset {
                rel_offset: unsafe { spec.offset.rel_offset }
                    + crate::util::metadata::side_metadata::metadata_bytes_per_chunk(
                        spec.log_min_obj_size,
                        spec.log_num_of_bits,
                    ),
            }
        }
    }
}

// Address and usize has the same layout, so we use usize for implementing these traits.

impl PartialEq for SideMetadataOffset {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.rel_offset == other.rel_offset }
    }
}
impl Eq for SideMetadataOffset {}

impl std::hash::Hash for SideMetadataOffset {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        unsafe { self.rel_offset }.hash(state);
    }
}

/// This struct stores all the side metadata specs for a policy. Generally a policy needs to know its own
/// side metadata spec as well as the plan's specs.
pub struct SideMetadataContext {
    // For plans
    pub global: Vec<SideMetadataSpec>,
    // For policies
    pub local: Vec<SideMetadataSpec>,
}

impl SideMetadataContext {
    #[cfg(not(feature = "global_alloc_bit"))]
    pub fn new_global_specs(specs: &[SideMetadataSpec]) -> Vec<SideMetadataSpec> {
        let mut ret = vec![];
        ret.extend_from_slice(specs);
        ret
    }

    #[cfg(feature = "global_alloc_bit")]
    pub fn new_global_specs(specs: &[SideMetadataSpec]) -> Vec<SideMetadataSpec> {
        let mut ret = vec![];
        ret.extend_from_slice(&[ALLOC_SIDE_METADATA_SPEC]);
        ret.extend_from_slice(specs);
        ret
    }

    pub fn get_local_specs(&self) -> &[SideMetadataSpec] {
        &self.local
    }

    /// Return the pages reserved for side metadata based on the data pages we used.
    // We used to use PageAccouting to count pages used in side metadata. However,
    // that means we always count pages while we may reserve less than a page each time.
    // This could lead to overcount. I think the easier way is to not account
    // when we allocate for sidemetadata, but to calculate the side metadata usage based on
    // how many data pages we use when reporting.
    pub fn calculate_reserved_pages(&self, data_pages: usize) -> usize {
        let mut total = 0;
        for spec in self.global.iter() {
            let rshift = addr_rshift(spec);
            total += (data_pages + ((1 << rshift) - 1)) >> rshift;
        }
        for spec in self.local.iter() {
            let rshift = addr_rshift(spec);
            total += (data_pages + ((1 << rshift) - 1)) >> rshift;
        }
        total
    }

    pub fn reset(&self) {}

    // ** NOTE: **
    //  Regardless of the number of bits in a metadata unit, we always represent its content as a word.

    /// Tries to map the required metadata space and returns `true` is successful.
    /// This can be called at page granularity.
    pub fn try_map_metadata_space(&self, start: Address, size: usize) -> Result<()> {
        debug!(
            "try_map_metadata_space({}, 0x{:x}, {}, {})",
            start,
            size,
            self.global.len(),
            self.local.len()
        );
        // Page aligned
        debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
        debug_assert!(size % BYTES_IN_PAGE == 0);
        self.map_metadata_internal(start, size, false)
    }

    /// Tries to map the required metadata address range, without reserving swap-space/physical memory for it.
    /// This will make sure the address range is exclusive to the caller. This should be called at chunk granularity.
    ///
    /// NOTE: Accessing addresses in this range will produce a segmentation fault if swap-space is not mapped using the `try_map_metadata_space` function.
    pub fn try_map_metadata_address_range(&self, start: Address, size: usize) -> Result<()> {
        debug!(
            "try_map_metadata_address_range({}, 0x{:x}, {}, {})",
            start,
            size,
            self.global.len(),
            self.local.len()
        );
        // Chunk aligned
        debug_assert!(start.is_aligned_to(BYTES_IN_CHUNK));
        debug_assert!(size % BYTES_IN_CHUNK == 0);
        self.map_metadata_internal(start, size, true)
    }

    /// The internal function to mmap metadata
    ///
    /// # Arguments
    /// * `start` - The starting address of the source data.
    /// * `size` - The size of the source data (in bytes).
    /// * `no_reserve` - whether to invoke mmap with a noreserve flag (we use this flag to quanrantine address range)
    fn map_metadata_internal(&self, start: Address, size: usize, no_reserve: bool) -> Result<()> {
        for spec in self.global.iter() {
            match try_mmap_contiguous_metadata_space(start, size, spec, no_reserve) {
                Ok(_) => {}
                Err(e) => return Result::Err(e),
            }
        }

        #[cfg(target_pointer_width = "32")]
        let mut lsize: usize = 0;

        for spec in self.local.iter() {
            // For local side metadata, we always have to reserve address space for all
            // local metadata required by all policies in MMTk to be able to calculate a constant offset for each local metadata at compile-time
            // (it's like assigning an ID to each policy).
            // As the plan is chosen at run-time, we will never know which subset of policies will be used during run-time.
            // We can't afford this much address space in 32-bits.
            // So, we switch to the chunk-based approach for this specific case.
            //
            // The global metadata is different in that for each plan, we can calculate its constant base addresses at compile-time.
            // Using the chunk-based approach will need the same address space size as the current not-chunked approach.
            #[cfg(target_pointer_width = "64")]
            {
                match try_mmap_contiguous_metadata_space(start, size, spec, no_reserve) {
                    Ok(_) => {}
                    Err(e) => return Result::Err(e),
                }
            }
            #[cfg(target_pointer_width = "32")]
            {
                lsize += metadata_bytes_per_chunk(spec.log_min_obj_size, spec.log_num_of_bits);
            }
        }

        #[cfg(target_pointer_width = "32")]
        if lsize > 0 {
            let max = BYTES_IN_CHUNK >> super::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;
            debug_assert!(
                lsize <= max,
                "local side metadata per chunk (0x{:x}) must be less than (0x{:x})",
                lsize,
                max
            );
            match try_map_per_chunk_metadata_space(start, size, lsize, no_reserve) {
                Ok(_) => {}
                Err(e) => return Result::Err(e),
            }
        }

        Ok(())
    }

    /// Unmap the corresponding metadata space or panic.
    ///
    /// Note-1: This function is only used for test and debug right now.
    ///
    /// Note-2: This function uses munmap() which works at page granularity.
    ///     If the corresponding metadata space's size is not a multiple of page size,
    ///     the actual unmapped space will be bigger than what you specify.
    #[cfg(test)]
    pub fn ensure_unmap_metadata_space(&self, start: Address, size: usize) {
        trace!("ensure_unmap_metadata_space({}, 0x{:x})", start, size);
        debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
        debug_assert!(size % BYTES_IN_PAGE == 0);

        for spec in self.global.iter() {
            ensure_munmap_contiguos_metadata_space(start, size, spec);
        }

        for spec in self.local.iter() {
            #[cfg(target_pointer_width = "64")]
            {
                ensure_munmap_contiguos_metadata_space(start, size, spec);
            }
            #[cfg(target_pointer_width = "32")]
            {
                ensure_munmap_chunked_metadata_space(start, size, spec);
            }
        }
    }
}

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
pub fn ensure_metadata_is_mapped(metadata_spec: &SideMetadataSpec, data_addr: Address) {
    let meta_start = address_to_meta_address(metadata_spec, data_addr).align_down(BYTES_IN_PAGE);

    debug!(
        "ensure_metadata_is_mapped({}).meta_start({})",
        data_addr, meta_start
    );

    memory::panic_if_unmapped(meta_start, BYTES_IN_PAGE);
}

#[inline(always)]
pub fn load_atomic(metadata_spec: &SideMetadataSpec, data_addr: Address, order: Ordering) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

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
    sanity::verify_load(metadata_spec, data_addr, res);

    res
}

#[inline(always)]
pub fn store_atomic(
    metadata_spec: &SideMetadataSpec,
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

    let bits_num_log = metadata_spec.log_num_of_bits;

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

#[inline(always)]
pub fn compare_exchange_atomic(
    metadata_spec: &SideMetadataSpec,
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

    let bits_num_log = metadata_spec.log_num_of_bits;

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
#[inline(always)]
pub fn fetch_add_atomic(
    metadata_spec: &SideMetadataSpec,
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

    let bits_num_log = metadata_spec.log_num_of_bits;

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
#[inline(always)]
pub fn fetch_sub_atomic(
    metadata_spec: &SideMetadataSpec,
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

    let bits_num_log = metadata_spec.log_num_of_bits;

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
#[inline(always)]
pub unsafe fn load(metadata_spec: &SideMetadataSpec, data_addr: Address) -> usize {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    let meta_addr = address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    let bits_num_log = metadata_spec.log_num_of_bits;

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
    sanity::verify_load(metadata_spec, data_addr, res);

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
#[inline(always)]
pub unsafe fn store(metadata_spec: &SideMetadataSpec, data_addr: Address, metadata: usize) {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

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

    #[cfg(feature = "extreme_assertions")]
    sanity::verify_store(metadata_spec, data_addr, metadata);
}

/// A byte array in side-metadata
pub struct MetadataByteArrayRef<const ENTRIES: usize> {
    #[cfg(feature = "extreme_assertions")]
    heap_range_start: Address,
    #[cfg(feature = "extreme_assertions")]
    spec: SideMetadataSpec,
    data: &'static [u8; ENTRIES],
}

impl<const ENTRIES: usize> MetadataByteArrayRef<ENTRIES> {
    /// Get a piece of metadata address range as a byte array.
    ///
    /// # Arguments
    ///
    /// * `metadata_spec` - The specification of the target side metadata.
    /// * `start` - The starting address of the heap range.
    /// * `bytes` - The size of the heap range.
    ///
    pub fn new(metadata_spec: &SideMetadataSpec, start: Address, bytes: usize) -> Self {
        debug_assert_eq!(
            metadata_spec.log_num_of_bits, LOG_BITS_IN_BYTE as usize,
            "Each heap entry should map to a byte in side-metadata"
        );
        debug_assert_eq!(
            bytes >> metadata_spec.log_min_obj_size,
            ENTRIES,
            "Heap range size and MetadataByteArray size does not match"
        );
        Self {
            #[cfg(feature = "extreme_assertions")]
            heap_range_start: start,
            #[cfg(feature = "extreme_assertions")]
            spec: *metadata_spec,
            // # Safety
            // The metadata memory is assumed to be mapped when accessing.
            data: unsafe { &*address_to_meta_address(metadata_spec, start).to_ptr() },
        }
    }

    /// Get the length of the array.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> usize {
        ENTRIES
    }

    /// Get a byte from the metadata byte array at the given index.
    #[inline(always)]
    #[allow(clippy::let_and_return)]
    pub fn get(&self, index: usize) -> u8 {
        #[cfg(feature = "extreme_assertions")]
        let _lock = sanity::SANITY_LOCK.lock().unwrap();
        let value = self.data[index];
        #[cfg(feature = "extreme_assertions")]
        {
            let data_addr = self.heap_range_start + (index << self.spec.log_min_obj_size);
            sanity::verify_load(&self.spec, data_addr, value as _);
        }
        value
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
pub fn bzero_metadata(metadata_spec: &SideMetadataSpec, start: Address, size: usize) {
    #[cfg(feature = "extreme_assertions")]
    let _lock = sanity::SANITY_LOCK.lock().unwrap();

    // yiluowei: Not Sure but this assertion seems too strict for Immix recycled lines
    #[cfg(not(feature = "global_alloc_bit"))]
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
                        metadata_spec.log_num_of_bits,
                    ),
                );
                next_data_chunk += BYTES_IN_CHUNK;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::metadata::side_metadata::SideMetadataContext;

    // offset is not used in these tests.
    pub const ZERO_OFFSET: SideMetadataOffset = SideMetadataOffset { rel_offset: 0 };

    #[test]
    fn calculate_reserved_pages_one_spec() {
        // 1 bit per 8 bytes - 1:64
        let spec = SideMetadataSpec {
            is_global: true,
            offset: ZERO_OFFSET,
            log_num_of_bits: 0,
            log_min_obj_size: 3,
        };
        let side_metadata = SideMetadataContext {
            global: vec![spec],
            local: vec![],
        };
        assert_eq!(side_metadata.calculate_reserved_pages(0), 0);
        assert_eq!(side_metadata.calculate_reserved_pages(63), 1);
        assert_eq!(side_metadata.calculate_reserved_pages(64), 1);
        assert_eq!(side_metadata.calculate_reserved_pages(65), 2);
        assert_eq!(side_metadata.calculate_reserved_pages(1024), 16);
    }

    #[test]
    fn calculate_reserved_pages_multi_specs() {
        // 1 bit per 8 bytes - 1:64
        let gspec = SideMetadataSpec {
            is_global: true,
            offset: ZERO_OFFSET,
            log_num_of_bits: 0,
            log_min_obj_size: 3,
        };
        // 2 bits per page - 2 / (4k * 8) = 1:16k
        let lspec = SideMetadataSpec {
            is_global: false,
            offset: ZERO_OFFSET,
            log_num_of_bits: 1,
            log_min_obj_size: 12,
        };
        let side_metadata = SideMetadataContext {
            global: vec![gspec],
            local: vec![lspec],
        };
        assert_eq!(side_metadata.calculate_reserved_pages(1024), 16 + 1);
    }
}
