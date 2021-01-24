use super::helpers::{self, *};
use crate::util::{constants, memory, Address};
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ** NOTE: **
//  Regardless of the number of bits in a metadata unit, we always represent its content as a word.

pub(super) const INVALID_SIDEMETADATA_ID: SideMetadataID = SideMetadataID(MAX_METADATA_BITS + 1);

// Starting from zero and increasing by one, this type works as a simple side metadata id
#[derive(Copy, Clone)]
pub struct SideMetadataID(usize);

impl SideMetadataID {
    pub const fn new() -> SideMetadataID {
        INVALID_SIDEMETADATA_ID
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

// `align[metadata_id]` is the minimum alignment of the source data for `metadata_id`
// `meta_bits_num_vec[metadata_id]` stores the number of bits requested for `metadata_id`
// `meta_base_addr_vec[metadata_id]` stores the starting address of the memory to be mapped for the bits of `metadata_id`
pub struct SideMetadata {
    pub(super) internal_mutex: Arc<Mutex<()>>,
    pub(super) align: Vec<usize>,
    pub(super) meta_bits_num_log_vec: Vec<usize>,
    pub(super) meta_base_addr_vec: Vec<Address>,
}

unsafe impl Sync for SideMetadata {}

lazy_static! {
    pub(super) static ref METADATA_SINGLETON: SideMetadata = SideMetadata {
        internal_mutex: Arc::new(Mutex::new(())),
        align: Vec::with_capacity(MAX_METADATA_ID),
        meta_bits_num_log_vec: Vec::with_capacity(MAX_METADATA_ID),
        meta_base_addr_vec: Vec::with_capacity(MAX_METADATA_ID),
    };
}

impl SideMetadata {
    // FIXME(Javad): check the possibility of a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    /// Requests a number of metadata bits and returns a `SideMetadataID`.
    /// This ID is used for the future references to these bits and allows choosing between bit sets (e.g. global and policy-specific bits, each have an ID).
    ///
    /// # Arguments
    ///
    /// * `number_of_bits` - The number of bits per source data unit (e.g. per object).
    ///     Currently, the maximum metadata size per data unit is a word (usize).
    ///
    /// * `log_min_data_size` - The log of minimum source data (e.g. object) size.
    ///     The minimum data granularity is a word, which means the minimum value of this argument is 2 in 32-bits, and 3 in 64 bits systems.
    ///
    pub fn request_meta_bits(number_of_bits: usize, log_min_data_size: usize) -> SideMetadataID {
        trace!(
            "request_meta_bits({}, {})",
            number_of_bits,
            log_min_data_size
        );
        assert!(
            number_of_bits.is_power_of_two() && number_of_bits <= MAX_METADATA_BITS,
            "number of metadata bits ({}) must be a power of two and <= {}.",
            number_of_bits,
            MAX_METADATA_BITS
        );
        assert!(
            METADATA_SINGLETON.meta_bits_num_log_vec.len() < MAX_METADATA_ID,
            "Too many (>{}) metadata bit-sets requested",
            MAX_METADATA_ID
        );
        assert!(
            log_min_data_size >= (constants::LOG_BYTES_IN_WORD as usize),
            "Alignment ({}) is less than minimum ({})",
            log_min_data_size,
            constants::LOG_BYTES_IN_WORD
        );

        let number_of_bits_log: usize = number_of_bits.trailing_zeros() as usize;

        // This lock protects the only critical section where write access to SideMetadata's
        // internal data occurs. All other functions only access Rust's thread-safe data structures
        let _guard = METADATA_SINGLETON.internal_mutex.lock().unwrap();

        let next_id = SideMetadataID(METADATA_SINGLETON.meta_bits_num_log_vec.len());
        unsafe {
            METADATA_SINGLETON.mut_self().align.push(log_min_data_size);
            METADATA_SINGLETON
                .mut_self()
                .meta_bits_num_log_vec
                .push(number_of_bits_log);
        }
        let next_base_addr = if next_id.as_usize() == 0 {
            METADATA_BASE_ADDRESS
        } else {
            METADATA_SINGLETON.meta_base_addr_vec[next_id.as_usize() - 1]
                + meta_space_size(SideMetadataID(next_id.as_usize() - 1))
        };

        unsafe {
            METADATA_SINGLETON
                .mut_self()
                .meta_base_addr_vec
                .push(next_base_addr);
        }

        next_id
    }

    /// Tries to map the required metadata space and returns `true` is successful.
    ///
    /// # Arguments
    ///
    /// * `start` - The starting address of the source data.
    ///
    /// * `size` - The size of the source data (in bytes).
    ///
    /// * `metadata_id` - The ID of the side metadata to map the space for.
    ///
    pub fn try_map_meta_space(start: Address, size: usize, metadata_id: SideMetadataID) -> bool {
        try_map_meta(start, size, metadata_id)
    }

    pub fn load_atomic(metadata_id: SideMetadataID, data_addr: Address) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "load_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *unsafe {
            METADATA_SINGLETON
                .meta_bits_num_log_vec
                .get_unchecked(metadata_id.as_usize())
        };

        if bits_num_log <= 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id) as u8;
            let mask = (((1usize << (1usize << bits_num_log)) - 1) << lshift) as u8;
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
                MAX_METADATA_BITS
            );
        }
    }

    pub fn store_atomic(metadata_id: SideMetadataID, data_addr: Address, metadata: usize) {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "store_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *unsafe {
            METADATA_SINGLETON
                .meta_bits_num_log_vec
                .get_unchecked(metadata_id.as_usize())
        };

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

            let mut old_val = unsafe { meta_addr.load::<u8>() };
            let mut new_val = (old_val & !mask) | ((metadata as u8) << lshift);

            while unsafe {
                meta_addr
                    .compare_exchange::<AtomicU8>(
                        old_val,
                        new_val,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
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
                MAX_METADATA_BITS
            );
        }
    }

    pub fn compare_exchange_atomic(
        metadata_id: SideMetadataID,
        data_addr: Address,
        old_metadata: usize,
        new_metadata: usize,
    ) -> bool {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "cmpxng_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *unsafe {
            METADATA_SINGLETON
                .meta_bits_num_log_vec
                .get_unchecked(metadata_id.as_usize())
        };

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

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
                MAX_METADATA_BITS
            );
        }
    }

    // same as Rust atomics, this wraps around on overflow
    pub fn fetch_add_atomic(metadata_id: SideMetadataID, data_addr: Address, val: usize) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "fetch_add_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *unsafe {
            METADATA_SINGLETON
                .meta_bits_num_log_vec
                .get_unchecked(metadata_id.as_usize())
        };

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

            let mut old_val = unsafe { meta_addr.load::<u8>() };
            let mut new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
            let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

            while unsafe {
                meta_addr
                    .compare_exchange::<AtomicU8>(
                        old_val,
                        new_val,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
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
                MAX_METADATA_BITS
            );
        }
    }

    // same as Rust atomics, this wraps around on overflow
    pub fn fetch_sub_atomic(metadata_id: SideMetadataID, data_addr: Address, val: usize) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "fetch_sub_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *unsafe {
            METADATA_SINGLETON
                .meta_bits_num_log_vec
                .get_unchecked(metadata_id.as_usize())
        };

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

            let mut old_val = unsafe { meta_addr.load::<u8>() };
            let mut new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
            let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

            while unsafe {
                meta_addr
                    .compare_exchange::<AtomicU8>(
                        old_val,
                        new_val,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
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
                MAX_METADATA_BITS
            );
        }
    }

    /// Non-atomic load of metadata.
    ///
    /// # Safety
    ///
    /// This is unsafe because:
    ///
    /// 1. Concurrent access to this operation is undefined behavior.
    /// 2. Interleaving Non-atomic and atomic operations is undefined behavior.
    ///
    pub unsafe fn load(metadata_id: SideMetadataID, data_addr: Address) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *METADATA_SINGLETON
            .meta_bits_num_log_vec
            .get_unchecked(metadata_id.as_usize());

        if bits_num_log <= 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;
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
                MAX_METADATA_BITS
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
    pub unsafe fn store(metadata_id: SideMetadataID, data_addr: Address, metadata: usize) {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
            "metadata_id ({}) out of range",
            metadata_id.as_usize()
        );
        let bits_num_log = *METADATA_SINGLETON
            .meta_bits_num_log_vec
            .get_unchecked(metadata_id.as_usize());

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

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
                MAX_METADATA_BITS
            );
        }
    }

    /// Bulk-zero a metadata space.
    ///
    /// # Arguments
    ///
    /// * `start` - The starting address of the data whose metadata is being zeroed.
    ///
    /// * `size` - The size (in bytes) of the source data.
    ///
    /// * `metadata_id` - The ID of the target side metadata.
    ///
    pub fn bzero_meta_space(start: Address, size: usize, metadata_id: SideMetadataID) {
        let meta_start = helpers::address_to_meta_address(start, metadata_id);
        let meta_end = helpers::address_to_meta_address(start + size, metadata_id);
        memory::zero(meta_start, meta_end.as_usize() - meta_start.as_usize());
    }
}

#[cfg(test)]
mod tests {
    use crate::util::constants;
    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::side_metadata::helpers;
    use crate::util::side_metadata::SideMetadata;
    use crate::util::test_util::serial_test;

    #[test]
    fn test_side_metadata_request_meta_bits() {
        serial_test(|| {
            for i in 0..5 {
                SideMetadata::request_meta_bits(1 << i, constants::LOG_BYTES_IN_WORD as usize);
            }
        });
    }

    #[test]
    fn test_side_metadata_try_map_meta_space_lt4kb() {
        let number_of_bits = 1;
        let number_of_bits_log = 0;
        let align = constants::LOG_BYTES_IN_WORD as usize;
        let space_size = 1;

        let metadata_id = SideMetadata::request_meta_bits(number_of_bits, align);
        assert!(SideMetadata::try_map_meta_space(
            vm_layout_constants::HEAP_START,
            space_size,
            metadata_id
        ));
        assert!(
            helpers::meta_page_is_mapped(helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START,
                metadata_id
            ))
            .unwrap()
        );
        assert!(
            helpers::meta_page_is_mapped(helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START + space_size,
                metadata_id
            ))
            .unwrap()
        );
        assert!(
            !helpers::meta_page_is_mapped(helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START
                    + (helpers::META_SPACE_PAGE_SIZE
                        << (align + constants::LOG_BITS_IN_WORD - number_of_bits_log)),
                metadata_id
            ))
            .unwrap()
        );
    }

    #[test]
    fn test_side_metadata_try_map_meta_space_gt4kb() {
        let number_of_bits = 8;
        let align = constants::LOG_BYTES_IN_WORD as usize;
        let space_size = helpers::META_SPACE_PAGE_SIZE * 64 + 1;

        let metadata_id = SideMetadata::request_meta_bits(number_of_bits, align);
        assert!(SideMetadata::try_map_meta_space(
            vm_layout_constants::HEAP_START,
            space_size,
            metadata_id
        ));
        assert!(
            helpers::meta_page_is_mapped(helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START,
                metadata_id
            ))
            .unwrap()
        );
        assert!(
            helpers::meta_page_is_mapped(helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START + space_size,
                metadata_id
            ))
            .unwrap()
        );
        assert!(!helpers::meta_page_is_mapped(
            helpers::address_to_meta_page_address(
                vm_layout_constants::HEAP_START + space_size,
                metadata_id
            ) + helpers::META_SPACE_PAGE_SIZE
        )
        .unwrap());
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_ge8bits() {
        let data_addr = vm_layout_constants::HEAP_START;
        let metadata_id =
            SideMetadata::request_meta_bits(16, constants::LOG_BYTES_IN_WORD as usize);
        SideMetadata::try_map_meta_space(data_addr, constants::BYTES_IN_PAGE as usize, metadata_id);

        let zero = SideMetadata::fetch_add_atomic(metadata_id, data_addr, 5);
        assert_eq!(zero, 0);

        let five = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(five, 5);

        let another_five = SideMetadata::fetch_sub_atomic(metadata_id, data_addr, 2);
        assert_eq!(another_five, 5);

        let three = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(three, 3);
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_4bits() {
        let data_addr = vm_layout_constants::HEAP_START;
        let metadata_id = SideMetadata::request_meta_bits(4, constants::LOG_BYTES_IN_WORD as usize);
        SideMetadata::try_map_meta_space(data_addr, constants::BYTES_IN_PAGE as usize, metadata_id);

        let zero = SideMetadata::fetch_add_atomic(metadata_id, data_addr, 5);
        assert_eq!(zero, 0);

        let five = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(five, 5);

        let another_five = SideMetadata::fetch_sub_atomic(metadata_id, data_addr, 2);
        assert_eq!(another_five, 5);

        let three = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(three, 3);
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_2bits() {
        let data_addr = vm_layout_constants::HEAP_START;
        let metadata_id = SideMetadata::request_meta_bits(2, constants::LOG_BYTES_IN_WORD as usize);
        SideMetadata::try_map_meta_space(data_addr, constants::BYTES_IN_PAGE as usize, metadata_id);

        let zero = SideMetadata::fetch_add_atomic(metadata_id, data_addr, 2);
        assert_eq!(zero, 0);

        let two = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(two, 2);

        let another_two = SideMetadata::fetch_sub_atomic(metadata_id, data_addr, 1);
        assert_eq!(another_two, 2);

        let one = SideMetadata::load_atomic(metadata_id, data_addr);
        assert_eq!(one, 1);
    }
}
