use super::*;
#[cfg(feature = "global_alloc_bit")]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::constants::{BYTES_IN_PAGE, LOG_BITS_IN_BYTE};
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::memory;
use crate::util::metadata::metadata_val_traits::*;
use crate::util::Address;
use num_traits::FromPrimitive;
use std::fmt;
use std::io::Result;
use std::sync::atomic::{AtomicU8, Ordering};

/// This struct stores the specification of a side metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SideMetadataSpec {
    pub name: &'static str,
    pub is_global: bool,
    pub offset: SideMetadataOffset,
    /// Number of bits needed per region. E.g. 0 = 1 bit, 1 = 2 bit.
    pub log_num_of_bits: usize,
    /// Number of bytes of the region. E.g. 3 = 8 bytes, 12 = 4096 bytes (page).
    pub log_bytes_in_region: usize,
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

    /// Return the upperbound offset for the side metadata. The next side metadata should be laid out at this offset.
    #[cfg(target_pointer_width = "64")]
    pub const fn upper_bound_offset(&self) -> SideMetadataOffset {
        debug_assert!(self.is_absolute_offset());
        SideMetadataOffset {
            addr: unsafe { self.offset.addr }
                .add(crate::util::metadata::side_metadata::metadata_address_range_size(self)),
        }
    }

    /// Return the upperbound offset for the side metadata. The next side metadata should be laid out at this offset.
    #[cfg(target_pointer_width = "32")]
    pub const fn upper_bound_offset(&self) -> SideMetadataOffset {
        if self.is_absolute_offset() {
            SideMetadataOffset {
                addr: unsafe { self.offset.addr }
                    .add(crate::util::metadata::side_metadata::metadata_address_range_size(self)),
            }
        } else {
            SideMetadataOffset {
                rel_offset: unsafe { self.offset.rel_offset }
                    + crate::util::metadata::side_metadata::metadata_bytes_per_chunk(
                        self.log_bytes_in_region,
                        self.log_num_of_bits,
                    ),
            }
        }
    }

    /// The upper bound address for metadata address computed for this global spec. The computed metadata address
    /// should never be larger than this address. Otherwise, we are accessing the metadata that is laid out
    /// after this spec. This spec must be a contiguous side metadata spec (which uses address
    /// as offset).
    pub const fn upper_bound_address_for_contiguous(&self) -> Address {
        debug_assert!(self.is_absolute_offset());
        unsafe { self.upper_bound_offset().addr }
    }

    /// The upper bound address for metadata address computed for this global spec. The computed metadata address
    /// should never be larger than this address. Otherwise, we are accessing the metadata that is laid out
    /// after this spec. This spec must be a chunked side metadata spec (which uses relative offset). Only 32 bit local
    /// side metadata uses chunked metadata.
    #[cfg(target_pointer_width = "32")]
    pub const fn upper_bound_address_for_chunked(&self, data_addr: Address) -> Address {
        debug_assert!(self.is_rel_offset());
        address_to_meta_chunk_addr(data_addr).add(unsafe { self.upper_bound_offset().rel_offset })
    }

    /// Used only for debugging.
    /// This panics if the required metadata is not mapped
    #[cfg(debug_assertions)]
    pub(crate) fn assert_metadata_mapped(&self, data_addr: Address) {
        let meta_start = address_to_meta_address(self, data_addr).align_down(BYTES_IN_PAGE);

        debug!(
            "ensure_metadata_is_mapped({}).meta_start({})",
            data_addr, meta_start
        );

        memory::panic_if_unmapped(meta_start, BYTES_IN_PAGE);
    }

    /// Used only for debugging.
    /// * Assert if the given MetadataValue type matches the spec.
    /// * Assert if the provided value is valid in the spec.
    #[cfg(debug_assertions)]
    fn assert_value_type<T: MetadataValue>(&self, val: Option<T>) {
        let log_b = self.log_num_of_bits;
        match log_b {
            _ if log_b < 3 => {
                assert_eq!(T::LOG2, 3);
                if let Some(v) = val {
                    assert!(
                        v.to_u8().unwrap() < (1 << (1 << log_b)),
                        "Input value {:?} is invalid for the spec {:?}",
                        v,
                        self
                    );
                }
            }
            3..=6 => assert_eq!(T::LOG2, log_b as u32),
            _ => unreachable!("side metadata > {}-bits is not supported", 1 << log_b),
        }
    }

    /// Check with the mmapper to see if side metadata is mapped for the spec for the data address.
    #[inline]
    pub(crate) fn is_mapped(&self, data_addr: Address) -> bool {
        use crate::util::heap::layout::Mmapper;
        use crate::MMAPPER;
        let meta_addr = address_to_meta_address(self, data_addr);
        MMAPPER.is_mapped_address(meta_addr)
    }

    /// This method is used for bulk zeroing side metadata for a data address range. As we cannot guarantee
    /// that the data address range can be mapped to whole metadata bytes, we have to deal with cases that
    /// we need to mask and zero certain bits in a metadata byte.
    /// The end address and the end bit are exclusive.
    pub(super) fn zero_meta_bits(
        meta_start_addr: Address,
        meta_start_bit: u8,
        meta_end_addr: Address,
        meta_end_bit: u8,
    ) {
        // Start/end is the same, we don't need to do anything.
        if meta_start_addr == meta_end_addr && meta_start_bit == meta_end_bit {
            return;
        }

        // zeroing bytes
        if meta_start_bit == 0 && meta_end_bit == 0 {
            memory::zero(meta_start_addr, meta_end_addr - meta_start_addr);
            return;
        }

        if meta_start_addr == meta_end_addr {
            // we are zeroing selected bits in one byte
            let mask: u8 = (u8::MAX << meta_end_bit) | !(u8::MAX << meta_start_bit); // Get a mask that the bits we need to zero are set to zero, and the other bits are 1.

            unsafe { meta_start_addr.as_ref::<AtomicU8>() }.fetch_and(mask, Ordering::SeqCst);
        } else if meta_start_addr + 1usize == meta_end_addr && meta_end_bit == 0 {
            // we are zeroing the rest bits in one byte
            let mask = !(u8::MAX << meta_start_bit); // Get a mask that the bits we need to zero are set to zero, and the other bits are 1.

            unsafe { meta_start_addr.as_ref::<AtomicU8>() }.fetch_and(mask, Ordering::SeqCst);
        } else {
            // zero bits in the first byte
            Self::zero_meta_bits(meta_start_addr, meta_start_bit, meta_start_addr + 1usize, 0);
            // zero bytes in the middle
            Self::zero_meta_bits(meta_start_addr + 1usize, 0, meta_end_addr, 0);
            // zero bits in the last byte
            Self::zero_meta_bits(meta_end_addr, 0, meta_end_addr, meta_end_bit);
        }
    }

    /// Bulk-zero a specific metadata for a chunk. Note that this method is more sophisiticated than a simple memset, especially in the following
    /// cases:
    /// * the metadata for the range includes partial bytes (a few bits in the same byte).
    /// * for 32 bits local side metadata, the side metadata is stored in discontiguous chunks, we will have to bulk zero for each chunk's side metadata.
    ///
    /// # Arguments
    ///
    /// * `start`: The starting address of a memory region. The side metadata starting from this data address will be zeroed.
    /// * `size`: The size of the memory region.
    ///
    pub fn bzero_metadata(&self, start: Address, size: usize) {
        #[cfg(feature = "extreme_assertions")]
        let _lock = sanity::SANITY_LOCK.lock().unwrap();

        #[cfg(feature = "extreme_assertions")]
        sanity::verify_bzero(self, start, size);

        // Zero for a contiguous side metadata spec. We can simply calculate the data end address, and
        // calculate the metadata address for the data end.
        let zero_contiguous = |data_start: Address, data_bytes: usize| {
            if data_bytes == 0 {
                return;
            }
            let meta_start = address_to_meta_address(self, data_start);
            let meta_start_shift = meta_byte_lshift(self, data_start);
            let meta_end = address_to_meta_address(self, data_start + data_bytes);
            let meta_end_shift = meta_byte_lshift(self, data_start + data_bytes);
            Self::zero_meta_bits(meta_start, meta_start_shift, meta_end, meta_end_shift);
        };

        // Zero for a discontiguous side metadata spec (chunked metadata). The side metadata for different
        // chunks are stored in discontiguous memory. For example, Chunk #2 follows Chunk #1, but the side metadata
        // for Chunk #2 does not immediately follow the side metadata for Chunk #1. So when we bulk zero metadata for Chunk #1,
        // we cannot zero up to the metadata address for the Chunk #2 start. Otherwise it may zero unrelated metadata
        // between the two chunks' metadata.
        // Instead, we compute how many bytes/bits we need to zero.
        // The data for which the metadata will be zeroed has to be in the same chunk.
        #[cfg(target_pointer_width = "32")]
        let zero_discontiguous = |data_start: Address, data_bytes: usize| {
            use crate::util::constants::BITS_IN_BYTE;
            if data_bytes == 0 {
                return;
            }

            debug_assert_eq!(
                data_start.align_down(BYTES_IN_CHUNK),
                (data_start + data_bytes - 1).align_down(BYTES_IN_CHUNK),
                "The data to be zeroed in discontiguous specs needs to be in the same chunk"
            );

            let meta_start = address_to_meta_address(self, data_start);
            let meta_start_shift = meta_byte_lshift(self, data_start);

            // How many bits we need to zero for data_bytes
            let meta_total_bits = (data_bytes >> self.log_bytes_in_region) << self.log_num_of_bits;
            let meta_delta_bytes = meta_total_bits >> LOG_BITS_IN_BYTE;
            let meta_delta_bits: u8 = (meta_total_bits % BITS_IN_BYTE) as u8;

            // Calculate the end byte/addr and end bit
            let (meta_end, meta_end_shift) = {
                let mut end_addr = meta_start + meta_delta_bytes;
                let mut end_bit = meta_start_shift + meta_delta_bits;
                if end_bit >= BITS_IN_BYTE as u8 {
                    end_bit -= BITS_IN_BYTE as u8;
                    end_addr += 1usize;
                }
                (end_addr, end_bit)
            };

            Self::zero_meta_bits(meta_start, meta_start_shift, meta_end, meta_end_shift);
        };

        if cfg!(target_pointer_width = "64") || self.is_global {
            zero_contiguous(start, size);
        }
        #[cfg(target_pointer_width = "32")]
        if !self.is_global {
            // per chunk policy-specific metadata for 32-bits targets
            let chunk_num = ((start + size).align_down(BYTES_IN_CHUNK)
                - start.align_down(BYTES_IN_CHUNK))
                / BYTES_IN_CHUNK;
            if chunk_num == 0 {
                zero_discontiguous(start, size);
            } else {
                let second_data_chunk = start.align_up(BYTES_IN_CHUNK);
                // bzero the first sub-chunk
                zero_discontiguous(start, second_data_chunk - start);

                let last_data_chunk = (start + size).align_down(BYTES_IN_CHUNK);
                // bzero the last sub-chunk
                zero_discontiguous(last_data_chunk, start + size - last_data_chunk);
                let mut next_data_chunk = second_data_chunk;

                // bzero all chunks in the middle
                while next_data_chunk != last_data_chunk {
                    zero_discontiguous(next_data_chunk, BYTES_IN_CHUNK);
                    next_data_chunk += BYTES_IN_CHUNK;
                }
            }
        }
    }

    /// This is a wrapper method for implementing side metadata access. It does nothing other than
    /// calling the access function with no overhead, but in debug builds,
    /// it includes multiple checks to make sure the access is sane.
    /// * check whether the given value type matches the number of bits for the side metadata.
    /// * check if the side metadata memory is mapped.
    /// * check if the side metadata content is correct based on a sanity map (only for extreme assertions).
    #[inline(always)]
    #[allow(unused_variables)] // data_addr/input is not used in release build
    fn side_metadata_access<T: MetadataValue, R: Copy, F: FnOnce() -> R, V: FnOnce(R)>(
        &self,
        data_addr: Address,
        input: Option<T>,
        access_func: F,
        verify_func: V,
    ) -> R {
        // With extreme assertions, we maintain a sanity table for each side metadata access. For whatever we store in
        // side metadata, we store in the sanity table. So we can use that table to check if its results are conssitent
        // with the actual side metadata.
        // To achieve this, we need to apply a lock when we access side metadata. This will hide some concurrency bugs,
        // but makes it possible for us to assert our side metadata implementation is correct.
        #[cfg(feature = "extreme_assertions")]
        let _lock = sanity::SANITY_LOCK.lock().unwrap();

        // A few checks
        #[cfg(debug_assertions)]
        {
            self.assert_value_type::<T>(input);
            self.assert_metadata_mapped(data_addr);
        }

        // Actual access to the side metadata
        let ret = access_func();

        // Verifying the side metadata: checks the result with the sanity table, or store some results to the sanity table
        verify_func(ret);

        ret
    }

    /// Non-atomic load of metadata.
    ///
    /// # Safety
    ///
    /// This is unsafe because:
    ///
    /// 1. Concurrent access to this operation is undefined behaviour.
    /// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
    #[inline(always)]
    pub unsafe fn load<T: MetadataValue>(&self, data_addr: Address) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            None,
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    let byte_val = meta_addr.load::<u8>();

                    FromPrimitive::from_u8((byte_val & mask) >> lshift).unwrap()
                } else {
                    meta_addr.load::<T>()
                }
            },
            |_v| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_load(self, data_addr, _v);
            },
        )
    }

    /// Non-atomic store of metadata.
    ///
    /// # Safety
    ///
    /// This is unsafe because:
    ///
    /// 1. Concurrent access to this operation is undefined behaviour.
    /// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
    #[inline(always)]
    pub unsafe fn store<T: MetadataValue>(&self, data_addr: Address, metadata: T) {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(metadata),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    let old_val = meta_addr.load::<u8>();
                    let new_val = (old_val & !mask) | (metadata.to_u8().unwrap() << lshift);

                    meta_addr.store::<u8>(new_val);
                } else {
                    meta_addr.store::<T>(metadata);
                }
            },
            |_| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_store(self, data_addr, metadata);
            },
        )
    }

    #[inline(always)]
    pub fn load_atomic<T: MetadataValue>(&self, data_addr: Address, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            None,
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(order) };
                    FromPrimitive::from_u8((byte_val & mask) >> lshift).unwrap()
                } else {
                    unsafe { T::load_atomic(meta_addr, order) }
                }
            },
            |_v| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_load(self, data_addr, _v);
            },
        )
    }

    #[inline(always)]
    pub fn store_atomic<T: MetadataValue>(&self, data_addr: Address, metadata: T, order: Ordering) {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(metadata),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    let metadata_u8 = metadata.to_u8().unwrap();
                    let _ = unsafe {
                        <u8 as MetadataValue>::fetch_update(meta_addr, order, order, |v: u8| {
                            Some((v & !mask) | (metadata_u8 << lshift))
                        })
                    };
                } else {
                    unsafe {
                        T::store_atomic(meta_addr, metadata, order);
                    }
                }
            },
            |_| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_store(self, data_addr, metadata);
            },
        )
    }

    #[inline(always)]
    pub fn compare_exchange_atomic<T: MetadataValue>(
        &self,
        data_addr: Address,
        old_metadata: T,
        new_metadata: T,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> std::result::Result<T, T> {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(new_metadata),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;

                    let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(success_order) };
                    let expected_old_byte =
                        (real_old_byte & !mask) | ((old_metadata.to_u8().unwrap()) << lshift);
                    let expected_new_byte =
                        (expected_old_byte & !mask) | ((new_metadata.to_u8().unwrap()) << lshift);

                    unsafe {
                        meta_addr.compare_exchange::<AtomicU8>(
                            expected_old_byte,
                            expected_new_byte,
                            success_order,
                            failure_order,
                        )
                    }
                    .map(|x| FromPrimitive::from_u8((x & mask) >> lshift).unwrap())
                    .map_err(|x| FromPrimitive::from_u8((x & mask) >> lshift).unwrap())
                } else {
                    unsafe {
                        T::compare_exchange(
                            meta_addr,
                            old_metadata,
                            new_metadata,
                            success_order,
                            failure_order,
                        )
                    }
                }
            },
            |_res| {
                #[cfg(feature = "extreme_assertions")]
                if _res.is_ok() {
                    sanity::verify_store(self, data_addr, new_metadata);
                }
            },
        )
    }

    /// This is used to implement fetch_add/sub for bits.
    /// For fetch_and/or, we don't necessarily need this method. We could directly do fetch_and/or on the u8.
    #[inline(always)]
    fn fetch_ops_on_bits<F: Fn(u8) -> u8>(
        &self,
        data_addr: Address,
        meta_addr: Address,
        set_order: Ordering,
        fetch_order: Ordering,
        update: F,
    ) -> u8 {
        let lshift = meta_byte_lshift(self, data_addr);
        let mask = meta_byte_mask(self) << lshift;

        let old_raw_byte = unsafe {
            <u8 as MetadataValue>::fetch_update(
                meta_addr,
                set_order,
                fetch_order,
                |raw_byte: u8| {
                    let old_val = (raw_byte & mask) >> lshift;
                    let new_val = update(old_val);
                    let new_raw_byte = (raw_byte & !mask) | ((new_val << lshift) & mask);
                    Some(new_raw_byte)
                },
            )
        }
        .unwrap();
        (old_raw_byte & mask) >> lshift
    }

    /// Wraps around on overflow.
    #[inline(always)]
    pub fn fetch_add_atomic<T: MetadataValue>(
        &self,
        data_addr: Address,
        val: T,
        order: Ordering,
    ) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(val),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                let bits_num_log = self.log_num_of_bits;
                if bits_num_log < 3 {
                    FromPrimitive::from_u8(self.fetch_ops_on_bits(
                        data_addr,
                        meta_addr,
                        order,
                        order,
                        |x: u8| x.wrapping_add(val.to_u8().unwrap()),
                    ))
                    .unwrap()
                } else {
                    unsafe { T::fetch_add(meta_addr, val, order) }
                }
            },
            |_old_val| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.wrapping_add(&val))
            },
        )
    }

    #[inline(always)]
    pub fn fetch_sub_atomic<T: MetadataValue>(
        &self,
        data_addr: Address,
        val: T,
        order: Ordering,
    ) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(val),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                if self.log_num_of_bits < 3 {
                    FromPrimitive::from_u8(self.fetch_ops_on_bits(
                        data_addr,
                        meta_addr,
                        order,
                        order,
                        |x: u8| x.wrapping_sub(val.to_u8().unwrap()),
                    ))
                    .unwrap()
                } else {
                    unsafe { T::fetch_sub(meta_addr, val, order) }
                }
            },
            |_old_val| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.wrapping_sub(&val))
            },
        )
    }

    #[inline(always)]
    pub fn fetch_and_atomic<T: MetadataValue>(
        &self,
        data_addr: Address,
        val: T,
        order: Ordering,
    ) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(val),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                if self.log_num_of_bits < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    // We do not need to use fetch_ops_on_bits(), we can just set irrelavent bits to 1, and do fetch_and
                    let rhs = (val.to_u8().unwrap() << lshift) | !mask;
                    let old_raw_byte =
                        unsafe { <u8 as MetadataValue>::fetch_and(meta_addr, rhs, order) };
                    let old_val = (old_raw_byte & mask) >> lshift;
                    FromPrimitive::from_u8(old_val).unwrap()
                } else {
                    unsafe { T::fetch_and(meta_addr, val, order) }
                }
            },
            |_old_val| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.bitand(val))
            },
        )
    }

    #[inline(always)]
    pub fn fetch_or_atomic<T: MetadataValue>(
        &self,
        data_addr: Address,
        val: T,
        order: Ordering,
    ) -> T {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            Some(val),
            || {
                let meta_addr = address_to_meta_address(self, data_addr);
                if self.log_num_of_bits < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;
                    // We do not need to use fetch_ops_on_bits(), we can just set irrelavent bits to 0, and do fetch_or
                    let rhs = (val.to_u8().unwrap() << lshift) & mask;
                    let old_raw_byte =
                        unsafe { <u8 as MetadataValue>::fetch_or(meta_addr, rhs, order) };
                    let old_val = (old_raw_byte & mask) >> lshift;
                    FromPrimitive::from_u8(old_val).unwrap()
                } else {
                    unsafe { T::fetch_or(meta_addr, val, order) }
                }
            },
            |_old_val| {
                #[cfg(feature = "extreme_assertions")]
                sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.bitor(val))
            },
        )
    }

    #[inline(always)]
    pub fn fetch_update_atomic<T: MetadataValue, F: FnMut(T) -> Option<T> + Copy>(
        &self,
        data_addr: Address,
        set_order: Ordering,
        fetch_order: Ordering,
        mut f: F,
    ) -> std::result::Result<T, T> {
        self.side_metadata_access::<T, _, _, _>(
            data_addr,
            None,
            move || -> std::result::Result<T, T> {
                let meta_addr = address_to_meta_address(self, data_addr);
                if self.log_num_of_bits < 3 {
                    let lshift = meta_byte_lshift(self, data_addr);
                    let mask = meta_byte_mask(self) << lshift;

                    unsafe {
                        <u8 as MetadataValue>::fetch_update(
                            meta_addr,
                            set_order,
                            fetch_order,
                            |raw_byte: u8| {
                                let old_val = (raw_byte & mask) >> lshift;
                                f(FromPrimitive::from_u8(old_val).unwrap()).map(|new_val| {
                                    (raw_byte & !mask)
                                        | ((new_val.to_u8().unwrap() << lshift) & mask)
                                })
                            },
                        )
                    }
                    .map(|x| FromPrimitive::from_u8((x & mask) >> lshift).unwrap())
                    .map_err(|x| FromPrimitive::from_u8((x & mask) >> lshift).unwrap())
                } else {
                    unsafe { T::fetch_update(meta_addr, set_order, fetch_order, f) }
                }
            },
            |_result| {
                #[cfg(feature = "extreme_assertions")]
                if let Ok(old_val) = _result {
                    println!("Ok({})", old_val);
                    sanity::verify_update::<T>(self, data_addr, old_val, f(old_val).unwrap())
                }
            },
        )
    }
}

impl fmt::Debug for SideMetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "SideMetadataSpec {} {{ \
            **is_global: {:?} \
            **offset: {} \
            **log_num_of_bits: 0x{:x} \
            **log_bytes_in_region: 0x{:x} \
            }}",
            self.name,
            self.is_global,
            unsafe {
                if self.is_absolute_offset() {
                    format!("0x{:x}", self.offset.addr)
                } else {
                    format!("0x{:x}", self.offset.rel_offset)
                }
            },
            self.log_num_of_bits,
            self.log_bytes_in_region
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
    pub const fn layout_after(spec: &SideMetadataSpec) -> SideMetadataOffset {
        spec.upper_bound_offset()
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
    #[allow(clippy::vec_init_then_push)] // allow this, as we conditionally push based on features.
    pub fn new_global_specs(specs: &[SideMetadataSpec]) -> Vec<SideMetadataSpec> {
        let mut ret = vec![];

        #[cfg(feature = "global_alloc_bit")]
        ret.push(ALLOC_SIDE_METADATA_SPEC);

        {
            use crate::policy::sft_map::SFTMap;
            if let Some(spec) = crate::mmtk::SFT_MAP.get_side_metadata() {
                if spec.is_global {
                    ret.push(*spec);
                }
            }
        }

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
                lsize += metadata_bytes_per_chunk(spec.log_bytes_in_region, spec.log_num_of_bits);
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
            bytes >> metadata_spec.log_bytes_in_region,
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
            let data_addr = self.heap_range_start + (index << self.spec.log_bytes_in_region);
            sanity::verify_load::<u8>(&self.spec, data_addr, value);
        }
        value
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
            name: "test_spec",
            is_global: true,
            offset: ZERO_OFFSET,
            log_num_of_bits: 0,
            log_bytes_in_region: 3,
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
            name: "gspec",
            is_global: true,
            offset: ZERO_OFFSET,
            log_num_of_bits: 0,
            log_bytes_in_region: 3,
        };
        // 2 bits per page - 2 / (4k * 8) = 1:16k
        let lspec = SideMetadataSpec {
            name: "lspec",
            is_global: false,
            offset: ZERO_OFFSET,
            log_num_of_bits: 1,
            log_bytes_in_region: 12,
        };
        let side_metadata = SideMetadataContext {
            global: vec![gspec],
            local: vec![lspec],
        };
        assert_eq!(side_metadata.calculate_reserved_pages(1024), 16 + 1);
    }

    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::test_util::{serial_test, with_cleanup};
    use paste::paste;

    fn test_side_metadata(
        log_bits: usize,
        f: impl Fn(&SideMetadataSpec, Address, Address) + std::panic::RefUnwindSafe,
    ) {
        serial_test(|| {
            let spec = SideMetadataSpec {
                name: "Test Spec $tname",
                is_global: true,
                offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                log_num_of_bits: log_bits,
                log_bytes_in_region: 12, // page size
            };
            let context = SideMetadataContext {
                global: vec![spec],
                local: vec![],
            };
            let mut sanity = SideMetadataSanity::new();
            sanity.verify_metadata_context("TestPolicy", &context);

            let data_addr = vm_layout_constants::HEAP_START;
            let meta_addr = address_to_meta_address(&spec, data_addr);
            with_cleanup(
                || {
                    let mmap_result = context.try_map_metadata_space(data_addr, BYTES_IN_PAGE);
                    assert!(mmap_result.is_ok());

                    f(&spec, data_addr, meta_addr);
                },
                || {
                    // Clear the metadata -- use u64 (max length we support)
                    assert!(log_bits <= 6);
                    let meta_ptr: *mut u64 = meta_addr.to_mut_ptr();
                    unsafe { *meta_ptr = 0 };

                    sanity::reset();
                },
            )
        })
    }

    fn max_value(log_bits: usize) -> u64 {
        (0..(1 << log_bits)).fold(0, |accum, x| accum + (1 << x))
    }
    #[test]
    fn test_max_value() {
        assert_eq!(max_value(0), 1);
        assert_eq!(max_value(1), 0b11);
        assert_eq!(max_value(2), 0b1111);
        assert_eq!(max_value(3), 255);
        assert_eq!(max_value(4), 65535);
    }

    macro_rules! test_side_metadata_access {
        ($tname: ident, $type: ty, $log_bits: expr) => {
            paste!{
                #[test]
                fn [<$tname _load>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();

                        // Initial value should be 0
                        assert_eq!(unsafe { spec.load::<$type>(data_addr) }, 0);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), 0);

                        // Set to max
                        let max_value: $type = max_value($log_bits) as _;
                        unsafe { spec.store::<$type>(data_addr, max_value); }
                        assert_eq!(unsafe { spec.load::<$type>(data_addr) }, max_value);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), max_value);
                        assert_eq!(unsafe { *meta_ptr }, max_value);
                    });
                }

                #[test]
                fn [<$tname _store>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;

                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 0 to the side metadata
                        unsafe { spec.store::<$type>(data_addr, 0); }
                        assert_eq!(unsafe { spec.load::<$type>(data_addr) }, 0);
                        // Only the affected bits are set to 0
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX & (!max_value));
                    });
                }

                #[test]
                fn [<$tname _atomic_store>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;

                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 0 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);
                        assert_eq!(unsafe { spec.load::<$type>(data_addr) }, 0);
                        // Only the affected bits are set to 0
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX & (!max_value));
                    });
                }

                #[test]
                fn [<$tname _compare_exchange_success>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 1 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 1, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(old_val, 1);

                        let new_val = 0;
                        let res = spec.compare_exchange_atomic::<$type>(data_addr, old_val, new_val, Ordering::SeqCst, Ordering::SeqCst);
                        assert!(res.is_ok());
                        assert_eq!(res.unwrap(), old_val, "old vals do not match");

                        let after_update = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(after_update, new_val);
                        // Only the affected bits are set to 0
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX & (!max_value));
                    });
                }

                #[test]
                fn [<$tname _compare_exchange_fail>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 1 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 1, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(old_val, 1);

                        // make old_val outdated
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);
                        let bits_before_cas = unsafe { *meta_ptr };

                        let new_val = 0;
                        let res = spec.compare_exchange_atomic::<$type>(data_addr, old_val, new_val, Ordering::SeqCst, Ordering::SeqCst);
                        assert!(res.is_err());
                        assert_eq!(res.err().unwrap(), 0);
                        let bits_after_cas = unsafe { *meta_ptr };
                        assert_eq!(bits_before_cas, bits_after_cas);
                    });
                }

                #[test]
                fn [<$tname _fetch_add_1>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 0 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        let old_val_from_fetch = spec.fetch_add_atomic::<$type>(data_addr, 1, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, 1);
                    });
                }

                #[test]
                fn [<$tname _fetch_add_max>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 0 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        let old_val_from_fetch = spec.fetch_add_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, max_value);
                    });
                }

                #[test]
                fn [<$tname _fetch_add_overflow>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store max to the side metadata
                        spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        // add 1 to max value will cause overflow and wrap around to 0
                        let old_val_from_fetch = spec.fetch_add_atomic::<$type>(data_addr, 1, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, 0);
                    });
                }

                #[test]
                fn [<$tname _fetch_sub_1>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 1 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 1, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        let old_val_from_fetch = spec.fetch_sub_atomic::<$type>(data_addr, 1, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, 0);
                    });
                }

                #[test]
                fn [<$tname _fetch_sub_max>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store max to the side metadata
                        spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        let old_val_from_fetch = spec.fetch_sub_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, 0);
                    });
                }

                #[test]
                fn [<$tname _fetch_sub_overflow>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store 0 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);

                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                        // sub 1 from 0 will cause overflow, and wrap around to max
                        let old_val_from_fetch = spec.fetch_sub_atomic::<$type>(data_addr, 1, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);

                        let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        assert_eq!(new_val, max_value);
                    });
                }

                #[test]
                fn [<$tname _fetch_and>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store all 1s to the side metadata
                        spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                        // max and max should be max
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let old_val_from_fetch = spec.fetch_and_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val, "old values do not match");
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), max_value, "load values do not match");
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX, "raw values do not match");

                        // max and last_bit_zero should last_bit_zero
                        let last_bit_zero = max_value - 1;
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let old_val_from_fetch = spec.fetch_and_atomic::<$type>(data_addr, last_bit_zero, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), last_bit_zero);
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX - 1);
                    });
                }

                #[test]
                fn [<$tname _fetch_or>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 0s
                        unsafe { *meta_ptr = 0; }
                        // Store 0 to the side metadata
                        spec.store_atomic::<$type>(data_addr, 0, Ordering::SeqCst);

                        // 0 or 0 should be 0
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let old_val_from_fetch = spec.fetch_or_atomic::<$type>(data_addr, 0, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), 0);
                        assert_eq!(unsafe { *meta_ptr }, 0);

                        // 0 and max should max
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let old_val_from_fetch = spec.fetch_or_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);
                        assert_eq!(old_val_from_fetch, old_val);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), max_value);
                        assert_eq!(unsafe { *meta_ptr }, max_value);
                    });
                }

                #[test]
                fn [<$tname _fetch_update_success>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store all 1s to the side metadata
                        spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                        // update from max to zero
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let fetch_res = spec.fetch_update_atomic::<$type, _>(data_addr, Ordering::SeqCst, Ordering::SeqCst, |_x: $type| Some(0));
                        assert!(fetch_res.is_ok());
                        assert_eq!(fetch_res.unwrap(), old_val);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), 0);
                        // Only the affected bits are set to 0
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX & (!max_value));
                    });
                }

                #[test]
                fn [<$tname _fetch_update_fail>]() {
                    test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                        let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                        let max_value: $type = max_value($log_bits) as _;
                        // Set the metadata byte(s) to all 1s
                        unsafe { *meta_ptr = <$type>::MAX; }
                        // Store all 1s to the side metadata
                        spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                        // update from max to zero
                        let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                        let fetch_res = spec.fetch_update_atomic::<$type, _>(data_addr, Ordering::SeqCst, Ordering::SeqCst, |_x: $type| None);
                        assert!(fetch_res.is_err());
                        assert_eq!(fetch_res.err().unwrap(), old_val);
                        assert_eq!(spec.load_atomic::<$type>(data_addr, Ordering::SeqCst), max_value);
                        // Only the affected bits are set to 0
                        assert_eq!(unsafe { *meta_ptr }, <$type>::MAX);
                    });
                }
            }
        }
    }

    test_side_metadata_access!(test_u1, u8, 0);
    test_side_metadata_access!(test_u2, u8, 1);
    test_side_metadata_access!(test_u4, u8, 2);
    test_side_metadata_access!(test_u8, u8, 3);
    test_side_metadata_access!(test_u16, u16, 4);
    test_side_metadata_access!(test_u32, u32, 5);
    test_side_metadata_access!(test_u64, u64, 6);
    test_side_metadata_access!(
        test_usize,
        usize,
        if cfg!(target_pointer_width = "64") {
            6
        } else if cfg!(target_pointer_width = "32") {
            5
        } else {
            unreachable!()
        }
    );
}
