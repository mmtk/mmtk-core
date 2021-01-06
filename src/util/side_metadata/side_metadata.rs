use crate::util::{constants, memory, Address};
use std::sync::atomic::{AtomicUsize, Ordering};
// use crate::util::heap::layout::vm_layout_constants;

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

#[cfg(target_pointer_width = "32")]
const METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
const METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0x0000_f000_0000_0000) };

#[cfg(target_pointer_width = "32")]
const MAX_HEAP_SIZE_LOG: usize = 32;
// FIXME: This must be updated if the heap layout changes
#[cfg(target_pointer_width = "64")]
const MAX_HEAP_SIZE_LOG: usize = 48;

const MAX_METADATA_BITS: usize = 16;
const SPACE_PER_META_BIT: usize = 2 << (MAX_HEAP_SIZE_LOG - constants::LOG_BITS_IN_WORD);
const META_SPACE_PAGE_SIZE: usize = constants::BYTES_IN_PAGE;

// Starting from zero and increasing by one, this type works as a simple side metadata id
#[derive(Copy, Clone)]
pub struct MetadataID(usize);

impl MetadataID {
    pub fn new() -> MetadataID {
        MetadataID(MAX_METADATA_BITS + 1)
    }
}

// `align` is the minimum alignment
// `meta_bits_num_vec[metadata_id]` stores the number of bits requested for `metadata_id`
// `meta_base_addr_vec[metadata_id]` stores the starting address of the memory to be mapped for the bits of `metadata_id`
// `meta_cursor_addr_vec[metadata_id]` stores the starting address of the unmapped memory for the bits of `metadata_id`. Its initial value is the base address.
pub struct SideMetadata {
    align: Vec<usize>,
    meta_bits_num_log_vec: Vec<usize>,
    meta_base_addr_vec: Vec<Address>,
    meta_cursor_addr_vec: Vec<Address>,
}

unsafe impl Sync for SideMetadata {}

lazy_static! {
    static ref METADATA_SINGLETON: SideMetadata = SideMetadata {
        align: Vec::with_capacity(MAX_METADATA_BITS),
        meta_bits_num_log_vec: Vec::with_capacity(MAX_METADATA_BITS),
        meta_base_addr_vec: Vec::with_capacity(MAX_METADATA_BITS),
        meta_cursor_addr_vec: Vec::with_capacity(MAX_METADATA_BITS)
    };
}

impl SideMetadata {
    // FIXME(Javad): check the possibility of a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }
    // pub fn setup(align: usize, number_of_bits_vec: Vec<usize>) -> Vec<MetadataID> {
    //     METADATA_SINGLETON.align = align;
    //     let res = Vec::<MetadataID>::new();
    //     for num in number_of_bits_vec {
    //         res.append(SideMetadata::add_meta_bits(num));
    //     }
    // }

    // We currently do not differentiate between global and policy-specific bits
    //
    // Adds the requested number of bits to the side metadata and returns an ID.
    // This ID is used for the future references to these bits
    // and allows choosing between bit sets (e.g. global vs. policy-specific).
    //
    // This function reserves the required memory
    pub fn add_meta_bits(number_of_bits: usize, align: usize) -> MetadataID {
        assert!(
            [1, 2, 4, 8, 16].contains(&number_of_bits),
            "number of metadata bits ({}) must be a power of two",
            number_of_bits
        );
        assert!(
            SideMetadata::total_meta_bits() + number_of_bits < MAX_METADATA_BITS,
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
            _ => unreachable!(),
        };
        let next_id = MetadataID(METADATA_SINGLETON.meta_bits_num_log_vec.len());
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
                + SideMetadata::meta_space_size(MetadataID(next_id.0 - 1))
        };
        unsafe {
            METADATA_SINGLETON
                .mut_self()
                .meta_base_addr_vec
                .push(next_base_addr);
            METADATA_SINGLETON
                .mut_self()
                .meta_cursor_addr_vec
                .push(next_base_addr);
        }
        memory::reserve_address_range(next_base_addr, SideMetadata::meta_space_size(next_id));

        next_id
    }

    pub fn add_space(start: Address, size: usize) -> bool {
        for i in 0..METADATA_SINGLETON.meta_bits_num_log_vec.len() {
            let metadata_id = MetadataID(i);
            // if the added space is not already covered by this metadata space
            let new_cursor = SideMetadata::address_to_meta_word_address(start + size, metadata_id);
            let old_cursor = METADATA_SINGLETON.meta_cursor_addr_vec[metadata_id.0];
            if new_cursor > old_cursor {
                // mmap the rounded-up additional size
                let mmap_size = SideMetadata::round_up_to_page_size(new_cursor - old_cursor);
                memory::dzmmap(old_cursor, mmap_size);
                unsafe {
                    METADATA_SINGLETON.mut_self().meta_cursor_addr_vec[metadata_id.0] =
                        old_cursor + mmap_size;
                }
            }
        }

        true
    }

    #[inline(always)]
    fn meta_space_size(metadata_id: MetadataID) -> usize {
        let actual_size = 1usize
            << (MAX_HEAP_SIZE_LOG
                - constants::LOG_BITS_IN_WORD
                - METADATA_SINGLETON.align[metadata_id.0]
                + METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0]);
        // final size is always a multiple of page size
        let final_size = SideMetadata::round_up_to_page_size(actual_size);

        final_size
    }

    fn total_meta_bits() -> usize {
        let mut sum: usize = 0;
        for bits_log in METADATA_SINGLETON.meta_bits_num_log_vec.iter() {
            sum += 1 << bits_log;
        }

        sum
    }

    #[inline(always)]
    fn round_up_to_page_size(size: usize) -> usize {
        if size % META_SPACE_PAGE_SIZE == 0 {
            size
        } else {
            // round-up the size to page size
            (size >> constants::LOG_BYTES_IN_PAGE + 1) << constants::LOG_BITS_IN_PAGE
        }
    }

    #[inline(always)]
    fn address_to_meta_word_address(addr: Address, metadata_id: MetadataID) -> Address {
        let offset = addr
            >> (METADATA_SINGLETON.align[metadata_id.0] + (constants::LOG_BYTES_IN_WORD as usize)
                - METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0]);
        // clear the lowest address bits
        unsafe {
            Address::from_usize(
                (METADATA_SINGLETON.meta_base_addr_vec[metadata_id.0] + offset)
                    >> (constants::LOG_BYTES_IN_WORD as usize)
                    << (constants::LOG_BYTES_IN_WORD as usize),
            )
        }
    }

    #[inline(always)]
    fn meta_word_lshift(addr: Address, metadata_id: MetadataID) -> usize {
        // I assume compilers are smart enough to optimize remainder to (2^n) operations
        let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0];
        let res = (((addr.as_usize() >> constants::LOG_BYTES_IN_WORD)
            % (constants::BITS_IN_WORD >> bits_num_log))
            << bits_num_log)
            - bits_num_log;

        res
    }

    pub fn set_metadata(metadata_id: MetadataID, data_addr: Address, metadata: usize) {
        let word_addr = SideMetadata::address_to_meta_word_address(data_addr, metadata_id);
        unsafe {
            word_addr.atomic_store::<AtomicUsize>(
                metadata << SideMetadata::meta_word_lshift(data_addr, metadata_id),
                Ordering::SeqCst,
            )
        };
    }

    pub fn get_metadata(metadata_id: MetadataID, data_addr: Address) -> usize {
        let word_addr = SideMetadata::address_to_meta_word_address(data_addr, metadata_id);
        let word = unsafe { word_addr.atomic_load::<AtomicUsize>(Ordering::SeqCst) };
        // e.g. when 3-bits metadata:
        // (1<<3) - 1 = 0b111 then 0b111 << lshist
        // will be the mask
        let lshist = SideMetadata::meta_word_lshift(data_addr, metadata_id);
        let mask =
            ((1 as usize) << METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.0] - 1) << lshist;

        (word & mask) >> lshist
    }
}
