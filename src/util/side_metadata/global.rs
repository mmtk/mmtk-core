use super::*;
use crate::util::constants::{BYTES_IN_PAGE, LOG_BYTES_IN_PAGE};
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::PageAccounting;
use crate::util::memory;
use crate::util::{constants, Address};
use std::fmt;
use std::io::Result;
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SideMetadataSpec {
    pub scope: SideMetadataScope,
    pub offset: usize,
    pub log_num_of_bits: usize,
    pub log_min_obj_size: usize,
}

impl fmt::Debug for SideMetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "SideMetadataSpec {{").unwrap();
        writeln!(f, "\tScope: {:?}", self.scope).unwrap();
        writeln!(f, "\toffset: 0x{:x}", self.offset).unwrap();
        writeln!(f, "\tlog_num_of_bits: 0x{:x}", self.log_num_of_bits).unwrap();
        writeln!(f, "\tlog_min_obj_size: 0x{:x}\n}}", self.log_min_obj_size)
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
    pub fn new_global_specs(specs: &[SideMetadataSpec]) -> Vec<SideMetadataSpec> {
        let mut ret = vec![];
        ret.extend_from_slice(specs);
        if cfg!(feature = "side_gc_header") {
            ret.push(crate::util::gc_byte::SIDE_GC_BYTE_SPEC);
        }
        ret
    }
}

pub struct SideMetadata {
    context: SideMetadataContext,
    accounting: PageAccounting,
}

impl SideMetadata {
    pub fn new(context: SideMetadataContext) -> SideMetadata {
        sanity::verify_metadata_context(&context).unwrap();

        Self {
            context,
            accounting: PageAccounting::new(),
        }
    }

    pub fn reserved_pages(&self) -> usize {
        self.accounting.get_reserved_pages()
    }

    // ** NOTE: **
    //  Regardless of the number of bits in a metadata unit, we always represent its content as a word.

    /// Tries to map the required metadata space and returns `true` is successful.
    /// This can be called at page granularity.
    pub fn try_map_metadata_space(&self, start: Address, size: usize) -> Result<()> {
        debug!(
            "try_map_metadata_space({}, 0x{:x}, {}, {})",
            start,
            size,
            self.context.global.len(),
            self.context.local.len()
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
            self.context.global.len(),
            self.context.local.len()
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
        for spec in self.context.global.iter() {
            match try_mmap_contiguous_metadata_space(start, size, spec, no_reserve) {
                Ok(mapped) => {
                    // We actually reserved memory
                    if !no_reserve {
                        self.accounting
                            .reserve_and_commit(mapped >> LOG_BYTES_IN_PAGE);
                    }
                }
                Err(e) => return Result::Err(e),
            }
        }

        #[cfg(target_pointer_width = "32")]
        let mut lsize: usize = 0;

        for spec in self.context.local.iter() {
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
                    Ok(mapped) => {
                        // We actually reserved memory
                        if !no_reserve {
                            self.accounting
                                .reserve_and_commit(mapped >> LOG_BYTES_IN_PAGE);
                        }
                    }
                    Err(e) => return Result::Err(e),
                }
            }
            #[cfg(target_pointer_width = "32")]
            {
                lsize += meta_bytes_per_chunk(spec.log_min_obj_size, spec.log_num_of_bits);
            }
        }

        #[cfg(target_pointer_width = "32")]
        if lsize > 0 {
            let max = BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;
            debug_assert!(
                lsize <= max,
                "local side metadata per chunk (0x{:x}) must be less than (0x{:x})",
                lsize,
                max
            );
            match try_map_per_chunk_metadata_space(start, size, lsize, no_reserve) {
                Ok(mapped) => {
                    // We actually reserved memory
                    if !no_reserve {
                        self.accounting
                            .reserve_and_commit(mapped >> LOG_BYTES_IN_PAGE);
                    }
                }
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
    pub fn ensure_unmap_metadata_space(&self, start: Address, size: usize) {
        trace!("ensure_unmap_metadata_space({}, 0x{:x})", start, size);
        debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));
        debug_assert!(size % BYTES_IN_PAGE == 0);

        for spec in self.context.global.iter() {
            let size = ensure_munmap_contiguos_metadata_space(start, size, spec);
            self.accounting.release(size >> LOG_BYTES_IN_PAGE);
        }

        for spec in self.context.local.iter() {
            #[cfg(target_pointer_width = "64")]
            {
                let size = ensure_munmap_contiguos_metadata_space(start, size, spec);
                self.accounting.release(size >> LOG_BYTES_IN_PAGE);
            }
            #[cfg(target_pointer_width = "32")]
            {
                let size = ensure_munmap_chunked_metadata_space(start, size, spec);
                self.accounting.release(size >> LOG_BYTES_IN_PAGE);
            }
        }
    }
}

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
pub fn ensure_metadata_is_mapped(metadata_spec: SideMetadataSpec, data_addr: Address) {
    let meta_start = address_to_meta_address(metadata_spec, data_addr).align_down(BYTES_IN_PAGE);

    debug!(
        "ensure_metadata_is_mapped({}).meta_start({})",
        data_addr, meta_start
    );

    memory::panic_if_unmapped(meta_start, BYTES_IN_PAGE);
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

    #[allow(clippy::let_and_return)]
    let res = if bits_num_log < 3 {
        let lshift = meta_byte_lshift(metadata_spec, data_addr);
        let mask = meta_byte_mask(metadata_spec) << lshift;

        let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(Ordering::SeqCst) };
        let expected_old_byte = (real_old_byte & !mask) | ((old_metadata as u8) << lshift);
        let expected_new_byte = (expected_old_byte & !mask) | ((new_metadata as u8) << lshift);

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
    };

    #[cfg(feature = "extreme_assertions")]
    sanity::store(metadata_spec, data_addr, new_metadata).unwrap();

    res
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_add_atomic(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> usize {
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
    };

    #[cfg(feature = "extreme_assertions")]
    match sanity::add(metadata_spec, data_addr, val) {
        Ok(ov) => {
            assert!(
                ov == old_val,
                "Expected old val (0x{:x}), but found (0x{:x})",
                ov,
                old_val
            );
        }
        Err(e) => {
            panic!("metadata sanity checker failed with {}", e);
        }
    }

    old_val
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_sub_atomic(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> usize {
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
    };

    #[cfg(feature = "extreme_assertions")]
    match sanity::sub(metadata_spec, data_addr, val) {
        Ok(ov) => {
            assert!(
                ov == old_val,
                "Expected old val (0x{:x}), but found (0x{:x})",
                ov,
                old_val
            );
        }
        Err(e) => {
            panic!("metadata sanity checker failed with {}", e);
        }
    }

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
pub unsafe fn load(metadata_spec: SideMetadataSpec, data_addr: Address) -> usize {
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
    match sanity::load(&metadata_spec, data_addr) {
        Ok(exp_res) => {
            assert!(
                exp_res == res,
                "Expected old val (0x{:x}), but found (0x{:x})",
                exp_res,
                res
            );
        }
        Err(e) => {
            panic!("metadata sanity checker failed with {}", e);
        }
    }

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

    #[cfg(feature = "extreme_assertions")]
    sanity::store(metadata_spec, data_addr, metadata).unwrap();
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

    #[cfg(feature = "extreme_assertions")]
    sanity::bzero(metadata_spec, start, size).unwrap();

    let meta_start = address_to_meta_address(metadata_spec, start);
    if cfg!(target_pointer_width = "64") || metadata_spec.scope.is_global() {
        memory::zero(
            meta_start,
            address_to_meta_address(metadata_spec, start + size) - meta_start,
        );
    }
    #[cfg(target_pointer_width = "32")]
    if !metadata_spec.scope.is_global() {
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
                    meta_bytes_per_chunk(
                        metadata_spec.log_min_obj_size,
                        metadata_spec.log_num_of_bits,
                    ),
                );
                next_data_chunk += BYTES_IN_CHUNK;
            }
        }
    }
}
