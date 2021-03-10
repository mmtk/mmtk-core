use super::constants::*;
use crate::util::{Address, constants, heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK};


#[inline(always)]
pub(crate) fn address_to_meta_chunk_addr(data_addr: Address) -> Address {
    LOCAL_SIDE_METADATA_BASE_ADDRESS
        + ((data_addr.as_usize() & !CHUNK_MASK) >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
}

#[inline(always)]
pub(crate) const fn meta_bytes_per_chunk(log_min_obj_size: usize, log_num_of_bits: usize) -> usize {
    1usize
        << (LOG_BYTES_IN_CHUNK - (constants::LOG_BITS_IN_BYTE as usize) - log_min_obj_size
            + log_num_of_bits)
}