use super::helpers::*;
use crate::util::{constants, Address};
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, Ordering};

// The following information about the target data is required:
//     1. minimum data alignment (in #Bytes),
//     2. global metadata size per data unit (in #bits),
//     3. policy specific metadata size per data unit (in #bits)
//
// FACTS:
// - Regardless of the number of bits in a metadata unit,
// we always represent its content as a word.
// - Policy-specific bits are required for all data units, so when a space
// grows, all bits are grown as well.
//

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
    pub(super) align: Vec<usize>,
    pub(super) meta_bits_num_log_vec: Vec<usize>,
    pub(super) meta_base_addr_vec: Vec<Address>,
}

unsafe impl Sync for SideMetadata {}

lazy_static! {
    pub(super) static ref METADATA_SINGLETON: SideMetadata = SideMetadata {
        align: Vec::with_capacity(MAX_METADATA_BITS),
        meta_bits_num_log_vec: Vec::with_capacity(MAX_METADATA_BITS),
        meta_base_addr_vec: Vec::with_capacity(MAX_METADATA_BITS),
    };
}

impl SideMetadata {
    // FIXME(Javad): check the possibility of a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    // Adds the requested number of bits to the side metadata and returns an ID.
    // This ID is used for the future references to these bits
    // and allows choosing between bit sets (e.g. global and policy-specific bits, each have an ID).
    //
    // Arguments:
    //
    // `number_of_bits`: is the number of bits per source data unit (e.g. per object).
    //   Currently, the maximum metadata size per data unit is a word (usize).
    //
    // `align`: is the minimum alignment of the source data.
    //   The minimum data granularity is a word, which means the minimum value of this argument is 2 in 32-bits, and 3 in 64 bits systems.
    pub fn request_meta_bits(number_of_bits: usize, align: usize) -> SideMetadataID {
        assert!(
            [1, 2, 4, 8, 16, 32].contains(&number_of_bits),
            "number of metadata bits ({}) must be a power of two",
            number_of_bits
        );
        assert!(
            number_of_bits <= MAX_METADATA_BITS,
            "Too many (>{}) metadata bits requested",
            MAX_METADATA_BITS
        );
        assert!(
            align >= (constants::LOG_BYTES_IN_WORD as usize),
            "Alignment ({}) is less than minimum ({})",
            align,
            constants::LOG_BYTES_IN_WORD
        );

        let number_of_bits_log: usize = match number_of_bits {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            16 => 4,
            32 => 5,
            _ => unreachable!(),
        };
        let next_id = SideMetadataID(METADATA_SINGLETON.meta_bits_num_log_vec.len());
        unsafe {
            METADATA_SINGLETON.mut_self().align.push(align);
            METADATA_SINGLETON
                .mut_self()
                .meta_bits_num_log_vec
                .push(number_of_bits_log);
        }
        let next_base_addr = if next_id.0 == 0 {
            METADATA_BASE_ADDRESS
        } else {
            METADATA_SINGLETON.meta_base_addr_vec[next_id.0 - 1]
                + meta_space_size(SideMetadataID(next_id.0 - 1))
        };

        unsafe {
            METADATA_SINGLETON
                .mut_self()
                .meta_base_addr_vec
                .push(next_base_addr);
        }

        next_id
    }

    pub fn ensure_meta_space_is_mapped(
        start: Address,
        size: usize,
        metadata_id: SideMetadataID,
    ) -> bool {
        ensure_meta_is_mapped(start, size, metadata_id)
    }

    pub fn load_atomic(metadata_id: SideMetadataID, data_addr: Address) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        debug_assert!(
            meta_page_is_mapped(address_to_meta_page_address(data_addr, metadata_id)).unwrap(),
            "load_atomic.metadata_addr({}) for data_addr({}) is not mapped",
            meta_addr,
            data_addr
        );

        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];
        if bits_num_log <= 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id) as u8;
            let mask = (((1usize << (1usize << bits_num_log)) - 1) << lshift) as u8;
            let byte_val = unsafe { meta_addr.atomic_load::<AtomicU8>(Ordering::SeqCst) };

            ((byte_val & mask) as usize) >> lshift
        } else if bits_num_log == 4 {
            unsafe { meta_addr.atomic_load::<AtomicU16>(Ordering::SeqCst) as usize }
        } else if bits_num_log == 5 {
            unsafe { meta_addr.atomic_load::<AtomicU32>(Ordering::SeqCst) as usize }
        } else {
            todo!("side metadata > 32-bits is not supported yet!")
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

        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];
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
        } else {
            todo!("side metadata > 32-bits is not supported yet!");
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

        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];

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
        } else {
            todo!("side metadata > 32-bits is not supported yet!")
        }
    }

    pub fn load(metadata_id: SideMetadataID, data_addr: Address) -> usize {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];

        if bits_num_log <= 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;
            let byte_val = unsafe { meta_addr.load::<u8>() };

            ((byte_val & mask) as usize) >> lshift
        } else if bits_num_log == 4 {
            unsafe { meta_addr.load::<u16>() as usize }
        } else if bits_num_log == 5 {
            unsafe { meta_addr.load::<u32>() as usize }
        } else {
            todo!("side metadata > 32-bits is not supported yet!")
        }
    }

    pub fn store(metadata_id: SideMetadataID, data_addr: Address, metadata: usize) {
        let meta_addr = address_to_meta_address(data_addr, metadata_id);
        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];

        if bits_num_log < 3 {
            let lshift = meta_byte_lshift(data_addr, metadata_id);
            let mask = ((1 << (1 << bits_num_log)) - 1) << lshift;

            let old_val = unsafe { meta_addr.load::<u8>() };
            let new_val = (old_val & !mask) | ((metadata as u8) << lshift);

            unsafe { meta_addr.store::<u8>(new_val) };
        } else if bits_num_log == 3 {
            unsafe { meta_addr.store::<u8>(metadata as u8) };
        } else if bits_num_log == 4 {
            unsafe { meta_addr.store::<u16>(metadata as u16) };
        } else if bits_num_log == 5 {
            unsafe { meta_addr.store::<u32>(metadata as u32) };
        } else {
            todo!("side metadata > 32-bits is not supported yet!");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::util::constants;
    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::side_metadata::helpers;
    use crate::util::side_metadata::SideMetadata;

    #[test]
    fn test_side_metadata_request_meta_bits() {
        for i in 0..5 {
            SideMetadata::request_meta_bits(1 << i, constants::LOG_BYTES_IN_WORD as usize);
        }
    }

    #[test]
    fn test_ensure_meta_space_is_mapped_lt4kb() {
        let number_of_bits = 1;
        let number_of_bits_log = 0;
        let align = constants::LOG_BYTES_IN_WORD as usize;
        let space_size = 1;

        let metadata_id = SideMetadata::request_meta_bits(number_of_bits, align);
        assert!(SideMetadata::ensure_meta_space_is_mapped(
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
    fn test_ensure_meta_space_is_mapped_gt4kb() {
        let number_of_bits = 8;
        let align = constants::LOG_BYTES_IN_WORD as usize;
        let space_size = helpers::META_SPACE_PAGE_SIZE * 64 + 1;

        let metadata_id = SideMetadata::request_meta_bits(number_of_bits, align);
        assert!(SideMetadata::ensure_meta_space_is_mapped(
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
                vm_layout_constants::HEAP_START + space_size,
                metadata_id
            ) + helpers::META_SPACE_PAGE_SIZE)
            .unwrap()
        );
    }
}
