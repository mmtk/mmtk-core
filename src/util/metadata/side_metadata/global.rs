use super::*;
#[cfg(feature = "global_alloc_bit")]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::constants::{BYTES_IN_PAGE, LOG_BITS_IN_BYTE};
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::memory;
use crate::util::metadata::only_available_on_64bits;
use crate::util::{constants, Address};
use crate::util::metadata::metadata_val_traits::*;
use num_traits::{FromPrimitive, ToPrimitive};
use std::fmt;
use std::io::Result;
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};

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
    /// Assert if the given MetadataValue type matches the spec.
    fn assert_value_type<T: MetadataValue>(&self) {
        let log_b = self.log_num_of_bits;
        match log_b {
            _ if log_b < 3 => assert_eq!(T::LOG2, 3),
            3..=6 => assert_eq!(T::LOG2, log_b as u32),
            _ => unreachable!("side metadata > {}-bits is not supported", 1 << log_b),
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
    pub fn bzero_metadata(&self, start: Address, size: usize) {
        #[cfg(feature = "extreme_assertions")]
        let _lock = sanity::SANITY_LOCK.lock().unwrap();

        // yiluowei: Not Sure but this assertion seems too strict for Immix recycled lines
        #[cfg(not(feature = "global_alloc_bit"))]
        debug_assert!(
            start.is_aligned_to(BYTES_IN_PAGE) && meta_byte_lshift(self, start) == 0
        );

        #[cfg(feature = "extreme_assertions")]
        sanity::verify_bzero(self, start, size);

        let meta_start = address_to_meta_address(self, start);
        if cfg!(target_pointer_width = "64") || self.is_global {
            memory::zero(
                meta_start,
                address_to_meta_address(self, start + size) - meta_start,
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
                            metadata_spec.log_bytes_in_region,
                            metadata_spec.log_num_of_bits,
                        ),
                    );
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
    fn side_metadata_access<T: MetadataValue, R: Copy, F: FnMut() -> R, V: FnMut(R)>(&self, data_addr: Address, mut access_func: F, mut verify_func: V) -> R {
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
            self.assert_value_type::<T>();
            self.assert_metadata_mapped(data_addr);
        }

        // Actual access to the side metadata
        let ret = access_func();

        // Verifying the side metadata: checks the result with the sanity table, or store some results to the sanity table
        verify_func(ret);

        return ret;
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
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
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
        }, |_v| {
            #[cfg(feature = "extreme_assertions")]
            sanity::typed_verify_load(self, data_addr, _v);
        })
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
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
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
        }, |_| {
            #[cfg(feature = "extreme_assertions")]
            sanity::typed_verify_store(self, data_addr, metadata);
        })
    }

    #[inline(always)]
    pub fn load_atomic<T: MetadataValue>(&self, data_addr: Address, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            let bits_num_log = self.log_num_of_bits;
            if bits_num_log < 3 {
                let lshift = meta_byte_lshift(self, data_addr);
                let mask = meta_byte_mask(self) << lshift;
                let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(order) };
                FromPrimitive::from_u8((byte_val & mask) >> lshift).unwrap()
            } else {
                T::load_atomic(meta_addr, order)
            }
        }, |_v| {
            #[cfg(feature = "extreme_assertions")]
            sanity::typed_verify_load(self, data_addr, _v);
        })
    }

    #[inline(always)]
    pub fn store_atomic<T: MetadataValue>(&self, data_addr: Address, metadata: T, order: Ordering) {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            let bits_num_log = self.log_num_of_bits;
            if bits_num_log < 3 {
                let lshift = meta_byte_lshift(self, data_addr);
                let mask = meta_byte_mask(self) << lshift;
                let metadata_u8 = metadata.to_u8().unwrap();
                let mut old_val = unsafe { meta_addr.load::<u8>() };
                let mut new_val = (old_val & !mask) | (metadata_u8 << lshift);

                while unsafe {
                    meta_addr
                        .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                        .is_err()
                } {
                    old_val = unsafe { meta_addr.load::<u8>() };
                    new_val = (old_val & !mask) | (metadata_u8 << lshift);
                }
            } else {
                T::store_atomic(meta_addr, metadata, order)
            }
        }, |_| {
            #[cfg(feature = "extreme_assertions")]
            sanity::typed_verify_store(self, data_addr, metadata);
        })
    }

    #[inline(always)]
    pub fn compare_exchange_atomic<T: MetadataValue>(&self, data_addr: Address, old_metadata: T, new_metadata: T, success_order: Ordering, failure_order: Ordering) -> bool {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            let bits_num_log = self.log_num_of_bits;
            if bits_num_log < 3 {
                let lshift = meta_byte_lshift(self, data_addr);
                let mask = meta_byte_mask(self) << lshift;

                let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(success_order) };
                let expected_old_byte = (real_old_byte & !mask) | ((old_metadata.to_u8().unwrap()) << lshift);
                let expected_new_byte = (expected_old_byte & !mask) | ((new_metadata.to_u8().unwrap()) << lshift);

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
            } else {
                T::compare_exchange(meta_addr, old_metadata, new_metadata, success_order, failure_order).is_ok()
            }
        }, |_success| {
            #[cfg(feature = "extreme_assertions")]
            if _success {
                sanity::typed_verify_store(self, data_addr, new_metadata);
            }
        })
    }

    #[inline(always)]
    fn fetch_update_bits<F: Fn(u8) -> u8>(&self, data_addr: Address, meta_addr: Address, set_order: Ordering, fetch_order: Ordering, update: F) -> u8 {
        let lshift = meta_byte_lshift(self, data_addr);
        let mask = meta_byte_mask(self) << lshift;
        let mut old_val = unsafe { meta_addr.load::<u8>() };
        let mut new_sub_val = update((old_val & mask) >> lshift) & (mask >> lshift);
        let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

        while unsafe {
            meta_addr
                .compare_exchange::<AtomicU8>(old_val, new_val, set_order, fetch_order)
                .is_err()
        } {
            old_val = unsafe { meta_addr.load::<u8>() };
            new_sub_val = update((old_val & mask) >> lshift) & (mask >> lshift);
            new_val = (old_val & !mask) | (new_sub_val << lshift);
        }

        FromPrimitive::from_u8(old_val & mask).unwrap()
    }

    #[inline(always)]
    pub fn fetch_add_atomic<T: MetadataValue>(&self, data_addr: Address, val: T, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            let bits_num_log = self.log_num_of_bits;
            if bits_num_log < 3 {
                // let lshift = meta_byte_lshift(self, data_addr);
                // let mask = meta_byte_mask(self) << lshift;
                // let val_u8 = val.to_u8().unwrap();

                // let mut old_val = unsafe { meta_addr.load::<u8>() };
                // let mut new_sub_val = (((old_val & mask) >> lshift) + val_u8) & (mask >> lshift);
                // let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

                // while unsafe {
                //     meta_addr
                //         .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                //         .is_err()
                // } {
                //     old_val = unsafe { meta_addr.load::<u8>() };
                //     new_sub_val = (((old_val & mask) >> lshift) + val_u8) & (mask >> lshift);
                //     new_val = (old_val & !mask) | (new_sub_val << lshift);
                // }

                // FromPrimitive::from_u8(old_val & mask).unwrap()
                FromPrimitive::from_u8(self.fetch_update_bits(data_addr, meta_addr, order, order, |x: u8| x + val.to_u8().unwrap())).unwrap()
            } else {
                T::fetch_add(meta_addr, val, order)
            }
        }, |_old_val| {
            #[cfg(feature = "extreme_assertions")]
            // sanity::typed_verify_add(self, data_addr, val, _old_val)
            sanity::verify_update::<T>(self, data_addr, _old_val, _old_val + val)
        })
    }

    #[inline(always)]
    pub fn fetch_sub_atomic<T: MetadataValue>(&self, data_addr: Address, val: T, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            if self.log_num_of_bits < 3 {
                // let lshift = meta_byte_lshift(self, data_addr);
                // let mask = meta_byte_mask(self) << lshift;
                // let val_u8 = val.to_u8().unwrap();

                // let mut old_val = unsafe { meta_addr.load::<u8>() };
                // let mut new_sub_val = (((old_val & mask) >> lshift) - val_u8) & (mask >> lshift);
                // let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

                // while unsafe {
                //     meta_addr
                //         .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
                //         .is_err()
                // } {
                //     old_val = unsafe { meta_addr.load::<u8>() };
                //     new_sub_val = (((old_val & mask) >> lshift) - val_u8) & (mask >> lshift);
                //     new_val = (old_val & !mask) | (new_sub_val << lshift);
                // }

                // FromPrimitive::from_u8(old_val & mask).unwrap()
                FromPrimitive::from_u8(self.fetch_update_bits(data_addr, meta_addr, order, order, |x: u8| x - val.to_u8().unwrap())).unwrap()
            } else {
                T::fetch_sub(meta_addr, val, order)
            }
        }, |_old_val| {
            #[cfg(feature = "extreme_assertions")]
            // sanity::typed_verify_sub(self, data_addr, val, _old_val)
            sanity::verify_update::<T>(self, data_addr, _old_val, _old_val - val)
        })
    }

    #[inline(always)]
    pub fn fetch_and_atomic<T: MetadataValue>(&self, data_addr: Address, val: T, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            if self.log_num_of_bits < 3 {
                FromPrimitive::from_u8(self.fetch_update_bits(data_addr, meta_addr, order, order, |x: u8| x & val.to_u8().unwrap())).unwrap()
            } else {
                T::fetch_and(meta_addr, val, order)
            }
        }, |_old_val| {
            #[cfg(feature = "extreme_assertions")]
            sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.bitand(val))
        })
    }

    #[inline(always)]
    pub fn fetch_or_atomic<T: MetadataValue>(&self, data_addr: Address, val: T, order: Ordering) -> T {
        self.side_metadata_access::<T, _, _, _>(data_addr, || {
            let meta_addr = address_to_meta_address(self, data_addr);
            if self.log_num_of_bits < 3 {
                FromPrimitive::from_u8(self.fetch_update_bits(data_addr, meta_addr, order, order, |x: u8| x | val.to_u8().unwrap())).unwrap()
            } else {
                T::fetch_and(meta_addr, val, order)
            }
        }, |_old_val| {
            #[cfg(feature = "extreme_assertions")]
            sanity::verify_update::<T>(self, data_addr, _old_val, _old_val.bitor(val))
        })
    }

    #[inline(always)]
    pub fn fetch_update_atomic<T: MetadataValue>(&self, data_addr: Address, set_order: Ordering, fetch_order: Ordering, mut f: impl FnMut(T) -> Option<T> + Copy) -> std::result::Result<T, T> {
        self.side_metadata_access::<T, _, _, _>(data_addr, move || -> std::result::Result<T, T> {
            let meta_addr = address_to_meta_address(self, data_addr);
            if self.log_num_of_bits < 3 {
                let lshift = meta_byte_lshift(self, data_addr);
                let mask = meta_byte_mask(self) << lshift;
                let mut old_val = unsafe { meta_addr.load::<u8>() };
                while let Some(next) = f(FromPrimitive::from_u8((old_val & mask) >> lshift).unwrap()) {
                    let new_val = (old_val & !mask) | ((next.to_u8().unwrap()) << lshift);
                    match unsafe {
                        meta_addr.compare_exchange::<AtomicU8>(old_val, new_val, set_order, fetch_order)
                    } {
                        x @ Ok(_) => {
                            return x.map(|y| FromPrimitive::from_u8(y).unwrap()).map_err(|y| FromPrimitive::from_u8(y).unwrap())
                        }
                        Err(next_prev) => old_val = next_prev,
                    }
                }
                Err(FromPrimitive::from_u8((old_val & mask) >> lshift).unwrap())
            } else {
                T::fetch_update(meta_addr, set_order, fetch_order, f)
            }
        }, |_result| {
            #[cfg(feature = "extreme_assertions")]
            if let Ok(old_val) = _result {
                sanity::verify_update::<T>(self, data_addr, old_val, f(old_val).unwrap())
            }
        })
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

// Used only for debugging
// Panics in the required metadata for data_addr is not mapped
// pub fn ensure_metadata_is_mapped(metadata_spec: &SideMetadataSpec, data_addr: Address) {
//     let meta_start = address_to_meta_address(metadata_spec, data_addr).align_down(BYTES_IN_PAGE);

//     debug!(
//         "ensure_metadata_is_mapped({}).meta_start({})",
//         data_addr, meta_start
//     );

//     memory::panic_if_unmapped(meta_start, BYTES_IN_PAGE);
// }

// #[inline(always)]
// pub fn load_atomic(metadata_spec: &SideMetadataSpec, data_addr: Address, order: Ordering) -> usize {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     let res = if bits_num_log <= 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;
//         let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(order) };

//         ((byte_val & mask) as usize) >> lshift
//     } else if bits_num_log == 4 {
//         unsafe { meta_addr.atomic_load::<AtomicU16>(order) as usize }
//     } else if bits_num_log == 5 {
//         unsafe { meta_addr.atomic_load::<AtomicU32>(order) as usize }
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({ unsafe { meta_addr.atomic_load::<AtomicU64>(order) as usize } })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     };

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_load(metadata_spec, data_addr, res);

//     res
// }

// #[inline(always)]
// pub fn store_atomic(
//     metadata_spec: &SideMetadataSpec,
//     data_addr: Address,
//     metadata: usize,
//     order: Ordering,
// ) {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     if bits_num_log < 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;

//         let mut old_val = unsafe { meta_addr.load::<u8>() };
//         let mut new_val = (old_val & !mask) | ((metadata as u8) << lshift);

//         while unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
//                 .is_err()
//         } {
//             old_val = unsafe { meta_addr.load::<u8>() };
//             new_val = (old_val & !mask) | ((metadata as u8) << lshift);
//         }
//     } else if bits_num_log == 3 {
//         unsafe { meta_addr.atomic_store::<AtomicU8>(metadata as u8, order) };
//     } else if bits_num_log == 4 {
//         unsafe { meta_addr.atomic_store::<AtomicU16>(metadata as u16, order) };
//     } else if bits_num_log == 5 {
//         unsafe { meta_addr.atomic_store::<AtomicU32>(metadata as u32, order) };
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({
//             unsafe { meta_addr.atomic_store::<AtomicU64>(metadata as u64, order) };
//         })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     }

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_store(metadata_spec, data_addr, metadata);
// }

// #[inline(always)]
// pub fn compare_exchange_atomic(
//     metadata_spec: &SideMetadataSpec,
//     data_addr: Address,
//     old_metadata: usize,
//     new_metadata: usize,
//     success_order: Ordering,
//     failure_order: Ordering,
// ) -> bool {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     debug!(
//         "compare_exchange_atomic({:?}, {}, {}, {})",
//         metadata_spec, data_addr, old_metadata, new_metadata
//     );
//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     #[allow(clippy::let_and_return)]
//     let res = if bits_num_log < 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;

//         let real_old_byte = unsafe { meta_addr.atomic_load::<AtomicU8>(success_order) };
//         let expected_old_byte = (real_old_byte & !mask) | ((old_metadata as u8) << lshift);
//         let expected_new_byte = (expected_old_byte & !mask) | ((new_metadata as u8) << lshift);

//         unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU8>(
//                     expected_old_byte,
//                     expected_new_byte,
//                     success_order,
//                     failure_order,
//                 )
//                 .is_ok()
//         }
//     } else if bits_num_log == 3 {
//         unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU8>(
//                     old_metadata as u8,
//                     new_metadata as u8,
//                     success_order,
//                     failure_order,
//                 )
//                 .is_ok()
//         }
//     } else if bits_num_log == 4 {
//         unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU16>(
//                     old_metadata as u16,
//                     new_metadata as u16,
//                     success_order,
//                     failure_order,
//                 )
//                 .is_ok()
//         }
//     } else if bits_num_log == 5 {
//         unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU32>(
//                     old_metadata as u32,
//                     new_metadata as u32,
//                     success_order,
//                     failure_order,
//                 )
//                 .is_ok()
//         }
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({
//             unsafe {
//                 meta_addr
//                     .compare_exchange::<AtomicU64>(
//                         old_metadata as u64,
//                         new_metadata as u64,
//                         success_order,
//                         failure_order,
//                     )
//                     .is_ok()
//             }
//         })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     };

//     #[cfg(feature = "extreme_assertions")]
//     if res {
//         sanity::verify_store(metadata_spec, data_addr, new_metadata);
//     }

//     res
// }

// same as Rust atomics, this wraps around on overflow
// #[inline(always)]
// pub fn fetch_add_atomic(
//     metadata_spec: &SideMetadataSpec,
//     data_addr: Address,
//     val: usize,
//     order: Ordering,
// ) -> usize {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     #[allow(clippy::let_and_return)]
//     let old_val = if bits_num_log < 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;

//         let mut old_val = unsafe { meta_addr.load::<u8>() };
//         let mut new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
//         let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

//         while unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
//                 .is_err()
//         } {
//             old_val = unsafe { meta_addr.load::<u8>() };
//             new_sub_val = (((old_val & mask) >> lshift) + (val as u8)) & (mask >> lshift);
//             new_val = (old_val & !mask) | (new_sub_val << lshift);
//         }

//         (old_val & mask) as usize
//     } else if bits_num_log == 3 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU8>()).fetch_add(val as u8, order) as usize }
//     } else if bits_num_log == 4 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU16>()).fetch_add(val as u16, order) as usize }
//     } else if bits_num_log == 5 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU32>()).fetch_add(val as u32, order) as usize }
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({
//             unsafe { (*meta_addr.to_ptr::<AtomicU64>()).fetch_add(val as u64, order) as usize }
//         })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     };

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_add(metadata_spec, data_addr, val, old_val);

//     old_val
// }

// same as Rust atomics, this wraps around on overflow
// #[inline(always)]
// pub fn fetch_sub_atomic(
//     metadata_spec: &SideMetadataSpec,
//     data_addr: Address,
//     val: usize,
//     order: Ordering,
// ) -> usize {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     #[allow(clippy::let_and_return)]
//     let old_val = if bits_num_log < 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;

//         let mut old_val = unsafe { meta_addr.load::<u8>() };
//         let mut new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
//         let mut new_val = (old_val & !mask) | (new_sub_val << lshift);

//         while unsafe {
//             meta_addr
//                 .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
//                 .is_err()
//         } {
//             old_val = unsafe { meta_addr.load::<u8>() };
//             new_sub_val = (((old_val & mask) >> lshift) - (val as u8)) & (mask >> lshift);
//             new_val = (old_val & !mask) | (new_sub_val << lshift);
//         }

//         (old_val & mask) as usize
//     } else if bits_num_log == 3 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU8>()).fetch_sub(val as u8, order) as usize }
//     } else if bits_num_log == 4 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU16>()).fetch_sub(val as u16, order) as usize }
//     } else if bits_num_log == 5 {
//         unsafe { (*meta_addr.to_ptr::<AtomicU32>()).fetch_sub(val as u32, order) as usize }
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({
//             unsafe { (*meta_addr.to_ptr::<AtomicU64>()).fetch_sub(val as u64, order) as usize }
//         })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     };

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_sub(metadata_spec, data_addr, val, old_val);

//     old_val
// }

/// Non-atomic load of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
// #[inline(always)]
// pub unsafe fn load(metadata_spec: &SideMetadataSpec, data_addr: Address) -> usize {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     #[allow(clippy::let_and_return)]
//     let res = if bits_num_log <= 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;
//         let byte_val = meta_addr.load::<u8>();

//         ((byte_val & mask) as usize) >> lshift
//     } else if bits_num_log == 4 {
//         meta_addr.load::<u16>() as usize
//     } else if bits_num_log == 5 {
//         meta_addr.load::<u32>() as usize
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({ meta_addr.load::<u64>() as usize })
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     };

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_load(metadata_spec, data_addr, res);

//     res
// }

/// Non-atomic store of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
// #[inline(always)]
// pub unsafe fn store(metadata_spec: &SideMetadataSpec, data_addr: Address, metadata: usize) {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     if cfg!(debug_assertions) {
//         ensure_metadata_is_mapped(metadata_spec, data_addr);
//     }

//     let bits_num_log = metadata_spec.log_num_of_bits;

//     if bits_num_log < 3 {
//         let lshift = meta_byte_lshift(metadata_spec, data_addr);
//         let mask = meta_byte_mask(metadata_spec) << lshift;

//         let old_val = meta_addr.load::<u8>();
//         let new_val = (old_val & !mask) | ((metadata as u8) << lshift);

//         meta_addr.store::<u8>(new_val);
//     } else if bits_num_log == 3 {
//         meta_addr.store::<u8>(metadata as u8);
//     } else if bits_num_log == 4 {
//         meta_addr.store::<u16>(metadata as u16);
//     } else if bits_num_log == 5 {
//         meta_addr.store::<u32>(metadata as u32);
//     } else if bits_num_log == 6 {
//         only_available_on_64bits!({ meta_addr.store::<u64>(metadata as u64) });
//     } else {
//         unreachable!(
//             "side metadata > {}-bits is not supported!",
//             constants::BITS_IN_WORD
//         );
//     }

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_store(metadata_spec, data_addr, metadata);
// }

// fn assert_int_type<T: Bits>(spec: &SideMetadataSpec) {
//     let log_b = spec.log_num_of_bits;
//     match log_b {
//         _ if log_b < 3 => assert_eq!(T::LOG2, 3),
//         3..=6 => assert_eq!(T::LOG2, log_b as u32),
//         _ => unreachable!("side metadata > {}-bits is not supported", 1 << log_b),
//     }
// }

// #[inline(always)]
// fn side_metadata_access<T: MetadataValue, R: Copy, F: Fn() -> R, V: Fn(R)>(spec: &SideMetadataSpec, data_addr: Address, access_func: F, verify_func: V) -> R {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     if cfg!(debug_assertions) {
//         let log_b = spec.log_num_of_bits;
//         match log_b {
//             _ if log_b < 3 => assert_eq!(T::LOG2, 3),
//             3..=6 => assert_eq!(T::LOG2, log_b as u32),
//             _ => unreachable!("side metadata > {}-bits is not supported", 1 << log_b),
//         }
//         ensure_metadata_is_mapped(spec, data_addr);
//     }

//     let ret = access_func();

//     verify_func(ret);

//     return ret;
// }

// #[inline(always)]
// pub unsafe fn typed_load<T: MetadataValue>(metadata_spec: &SideMetadataSpec, data_addr: Address) -> T {
//     // #[cfg(feature = "extreme_assertions")]
//     // let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     // let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//     // if cfg!(debug_assertions) {
//     //     ensure_metadata_is_mapped(metadata_spec, data_addr);
//     // }

//     // let bits_num_log = metadata_spec.log_num_of_bits;

//     // #[allow(clippy::let_and_return)]
//     // let res = if bits_num_log <= 3 {
//     //     let lshift = meta_byte_lshift(metadata_spec, data_addr);
//     //     let mask = meta_byte_mask(metadata_spec) << lshift;
//     //     let byte_val = meta_addr.load::<u8>();

//     //     ((byte_val & mask) as usize) >> lshift
//     // } else if bits_num_log == 4 {
//     //     meta_addr.load::<u16>() as usize
//     // } else if bits_num_log == 5 {
//     //     meta_addr.load::<u32>() as usize
//     // } else if bits_num_log == 6 {
//     //     only_available_on_64bits!({ meta_addr.load::<u64>() as usize })
//     // } else {
//     //     unreachable!(
//     //         "side metadata > {}-bits is not supported!",
//     //         constants::BITS_IN_WORD
//     //     );
//     // };

//     // #[cfg(feature = "extreme_assertions")]
//     // sanity::verify_load(metadata_spec, data_addr, res);

//     // res
//     side_metadata_access::<T, _, _, _>(metadata_spec, data_addr, || {
//         let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//         let bits_num_log = metadata_spec.log_num_of_bits;
//         if bits_num_log < 3 {
//             let lshift = meta_byte_lshift(metadata_spec, data_addr);
//             let mask = meta_byte_mask(metadata_spec) << lshift;
//             let byte_val = meta_addr.load::<u8>();

//             num_traits::FromPrimitive::from_u8((byte_val & mask) >> lshift).unwrap()
//         } else {
//             meta_addr.load::<T>()
//         }
//     }, |_v| {
//         #[cfg(feature = "extreme_assertions")]
//         sanity::typed_verify_load(metadata_spec, data_addr, _v);
//     })
// }

// #[inline(never)]
// pub unsafe fn typed_store<T: MetadataValue>(metadata_spec: &SideMetadataSpec, data_addr: Address, metadata: T) {
//     side_metadata_access::<T, _, _, _>(metadata_spec, data_addr, || {
//         let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//         let bits_num_log = metadata_spec.log_num_of_bits;

//         if bits_num_log < 3 {
//             let lshift = meta_byte_lshift(metadata_spec, data_addr);
//             let mask = meta_byte_mask(metadata_spec) << lshift;

//             let old_val: u8 = meta_addr.load::<u8>();
//             let new_val = (old_val & !mask) | (metadata.to_u8().unwrap() << lshift);

//             meta_addr.store::<u8>(new_val);
//         } else {
//             meta_addr.store::<T>(metadata);
//         }
//     }, |_| {
//         #[cfg(feature = "extreme_assertions")]
//         sanity::typed_verify_store(metadata_spec, data_addr, metadata);
//     })
// }

// #[inline(always)]
// pub fn typed_store_atomic<T: MetadataValue>(
//     metadata_spec: &SideMetadataSpec,
//     data_addr: Address,
//     metadata: <T::AtomicType as MetadataAtomic>::NonAtomicType,
//     order: Ordering,
// ) {
//     side_metadata_access::<T, _, _, _>(metadata_spec, data_addr, || {
//         let meta_addr = address_to_meta_address(metadata_spec, data_addr);
//         let bits_num_log = metadata_spec.log_num_of_bits;
//         if bits_num_log < 3 {
//             let lshift = meta_byte_lshift(metadata_spec, data_addr);
//             let mask = meta_byte_mask(metadata_spec) << lshift;
//             let metadata_u8 = metadata.to_u8().unwrap();

//             let mut old_val = unsafe { meta_addr.load::<u8>() };
//             let mut new_val = (old_val & !mask) | (metadata_u8 << lshift);

//             while unsafe {
//                 meta_addr
//                     .compare_exchange::<AtomicU8>(old_val, new_val, order, order)
//                     .is_err()
//             } {
//                 old_val = unsafe { meta_addr.load::<u8>() };
//                 new_val = (old_val & !mask) | (metadata_u8 << lshift);
//             }
//         } else {
//             unsafe { meta_addr.as_ref::<T::AtomicType>() }.store(metadata, order)
//         }
//     }, |_| {
//         #[cfg(feature = "extreme_assertions")]
//         sanity::typed_verify_store(metadata_spec, data_addr, metadata);
//     })
// }

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
            sanity::typed_verify_load::<u8>(&self.spec, data_addr, value);
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
// pub fn bzero_metadata(metadata_spec: &SideMetadataSpec, start: Address, size: usize) {
//     #[cfg(feature = "extreme_assertions")]
//     let _lock = sanity::SANITY_LOCK.lock().unwrap();

//     // yiluowei: Not Sure but this assertion seems too strict for Immix recycled lines
//     #[cfg(not(feature = "global_alloc_bit"))]
//     debug_assert!(
//         start.is_aligned_to(BYTES_IN_PAGE) && meta_byte_lshift(metadata_spec, start) == 0
//     );

//     #[cfg(feature = "extreme_assertions")]
//     sanity::verify_bzero(metadata_spec, start, size);

//     let meta_start = address_to_meta_address(metadata_spec, start);
//     if cfg!(target_pointer_width = "64") || metadata_spec.is_global {
//         memory::zero(
//             meta_start,
//             address_to_meta_address(metadata_spec, start + size) - meta_start,
//         );
//     }
//     #[cfg(target_pointer_width = "32")]
//     if !metadata_spec.is_global {
//         // per chunk policy-specific metadata for 32-bits targets
//         let chunk_num = ((start + size).align_down(BYTES_IN_CHUNK)
//             - start.align_down(BYTES_IN_CHUNK))
//             / BYTES_IN_CHUNK;
//         if chunk_num == 0 {
//             memory::zero(
//                 meta_start,
//                 address_to_meta_address(metadata_spec, start + size) - meta_start,
//             );
//         } else {
//             let second_data_chunk = start.align_up(BYTES_IN_CHUNK);
//             // bzero the first sub-chunk
//             memory::zero(
//                 meta_start,
//                 address_to_meta_address(metadata_spec, second_data_chunk) - meta_start,
//             );
//             let last_data_chunk = (start + size).align_down(BYTES_IN_CHUNK);
//             let last_meta_chunk = address_to_meta_address(metadata_spec, last_data_chunk);
//             // bzero the last sub-chunk
//             memory::zero(
//                 last_meta_chunk,
//                 address_to_meta_address(metadata_spec, start + size) - last_meta_chunk,
//             );
//             let mut next_data_chunk = second_data_chunk;
//             // bzero all chunks in the middle
//             while next_data_chunk != last_data_chunk {
//                 memory::zero(
//                     address_to_meta_address(metadata_spec, next_data_chunk),
//                     metadata_bytes_per_chunk(
//                         metadata_spec.log_bytes_in_region,
//                         metadata_spec.log_num_of_bits,
//                     ),
//                 );
//                 next_data_chunk += BYTES_IN_CHUNK;
//             }
//         }
//     }
// }

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

    use crate::util::test_util::{serial_test, with_cleanup};
    use crate::util::heap::layout::vm_layout_constants;
    use paste::paste;

    fn test_side_metadata(log_bits: usize, f: impl Fn(&SideMetadataSpec, Address, Address) + std::panic::RefUnwindSafe) {
        serial_test(|| {
            let spec = SideMetadataSpec {
                name: "Test Spec $tname",
                is_global: true,
                offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                log_num_of_bits: log_bits,
                log_bytes_in_region: 12 // page size
            };
            let num_of_bits = 1 << log_bits;
            let context = SideMetadataContext { global: vec![spec], local: vec![] };
            let mut sanity = SideMetadataSanity::new();
            sanity.verify_metadata_context("TestPolicy", &context);

            let data_addr = vm_layout_constants::HEAP_START;
            let meta_addr = address_to_meta_address(&spec, data_addr);

            with_cleanup(
                || {
                    let mmap_result = context.try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE);
                    assert!(mmap_result.is_ok());

                    f(&spec, data_addr, meta_addr);
                },
                || {
                    // Clear the metadata -- use u64 (max length we support)
                    assert!(log_bits <= 6);
                    let meta_ptr: *mut u64 = meta_addr.to_mut_ptr();
                    unsafe { *meta_ptr = 0 };

                    sanity::reset();
                }
            )
        })
    }

    fn max_value(log_bits: usize) -> usize {
        (0..(1 << log_bits)).fold(0, |accum, x| { accum + (1 << x) })
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
                        assert!(res);

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
                        assert!(!res);
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

                // #[test]
                // fn [<$tname _fetch_add_overflow>]() {
                //     test_side_metadata($log_bits, |spec, data_addr, meta_addr| {
                //         let meta_ptr: *mut $type = meta_addr.to_mut_ptr();
                //         let max_value: $type = max_value($log_bits) as _;
                //         // Set the metadata byte(s) to all 1s
                //         unsafe { *meta_ptr = <$type>::MAX; }
                //         // Store max to the side metadata
                //         spec.store_atomic::<$type>(data_addr, max_value, Ordering::SeqCst);

                //         let old_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);

                //         // add 1 to max value will cause overflow
                //         let old_val_from_fetch = spec.fetch_add_atomic::<$type>(data_addr, 1, Ordering::SeqCst);
                //         assert_eq!(old_val_from_fetch, old_val);

                //         let new_val = spec.load_atomic::<$type>(data_addr, Ordering::SeqCst);
                //         assert_eq!(new_val, 0);
                //         assert_eq!(unsafe { *meta_ptr }, <$type>::MAX & (!max_value));
                //     });
                // }
            }
        }
    }

    test_side_metadata_access!(test_1bit, u8, 0);
    test_side_metadata_access!(test_2bits, u8, 1);
    test_side_metadata_access!(test_u8, u8, 3);
    test_side_metadata_access!(test_u64, u64, 6);
}
