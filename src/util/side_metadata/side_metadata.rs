use crate::util::{Address, constants, memory};

// The following information about the target data is required:
//     1. minimum data alignment (in #Bytes),
//     2. metadata size per data unit (in #bits),
//     3. start and end address of the data.

pub struct SideMetadata {
    min_align: usize,
    bits_per_data: usize,
    data_base_addr: Address,
    max_data_size:  usize,
    metadata_base: Address,
}

impl SideMetadata {
    pub fn new(
        min_align: usize,
        bits_per_data: usize,
        data_base_addr: Address,
        max_data_size:  usize,
    ) -> Self {
        SideMetadata {
            min_align,
            bits_per_data,
            data_base_addr,
            max_data_size,
            metadata_base: memory::reserve_vm_address_range(
                max_data_size * 
                bits_per_data / 
                min_align /
                constants::BITS_IN_BYTE
            )
        }
    }

    // FIXME: somehow mmap the required portion of the metadata's address range
    pub fn add_space(start: Address, size: usize) {}
    
    // FIXME: fix the metadata type to generic, probably some user-defined <T>
    pub fn set_metadata(data_addr: Address, metadata: usize) {}
    pub fn get_metadata(data_addr: Address) -> usize {}
}