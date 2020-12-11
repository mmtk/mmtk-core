use crate::util::{Address, constants, memory};

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
const METADATA_BASE_ADDRESS: Address = Address::from_usize(0);
#[cfg(target_pointer_width = "64")]
const METADATA_BASE_ADDRESS: Address = Address::from_usize(0x0000f00000000000);
const MAX_METADATA_BITS: usize = 16;

type MetadataID = usize;

pub struct SideMetadata {
    align: usize,
    meta_bits_num_vec: Vec<usize>,
    meta_base_addr_vec: Vec<Address>,
}

impl SideMetadata {
    pub fn new(
        align: usize,
    ) -> Self {
        SideMetadata {
            align,
            meta_bits_num_vec: Vec::with_capacity(MAX_METADATA_BITS),
            meta_base_addr_vec: Vec::with_capacity(MAX_METADATA_BITS)
        }
    }

    // TODO: Do we want the number_of_bits to be a power of 2? LOOKS YES
    pub fn add_meta_bits(&mut self, number_of_bits: usize) -> MetadataID {
        debug_assert!(
            self.meta_bits_num_vec.iter().sum() + number_of_bits < MAX_METADATA_BITS,
            "Too many (>{}) metadata bits requested", MAX_METADATA_BITS
        );
        let res = self.meta_bits_vec.len();
        self.meta_bits_num_vec.append(number_of_bits);
        self.meta_base_addr_vec.append(other);
    }

    // FIXME: somehow mmap the required portion of the metadata's address range
    pub fn add_space(start: Address, size: usize) {}
    
    // FIXME: fix the metadata type to generic, probably some user-defined <T>
    pub fn set_metadata(data_addr: Address, metadata: usize) {}
    pub fn get_metadata(data_addr: Address) -> usize {}
}