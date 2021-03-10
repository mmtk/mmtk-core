use super::*;
use crate::util::heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE;
use crate::util::{constants, Address};

#[inline(always)]
pub(crate) fn address_to_meta_address(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let log_bits_num = metadata_spec.log_num_of_bits as i32;
    let log_min_obj_size = metadata_spec.log_min_obj_size as i32;

    debug_assert!(
        data_addr.is_aligned_to(1 << log_min_obj_size),
        "data_addr must be log_aligned to (0x{:x})",
        log_min_obj_size
    );

    let rshift = (constants::LOG_BITS_IN_BYTE as i32) + log_min_obj_size - log_bits_num;

    // policy-specific side metadata is per chunk in 32-bit targets
    if cfg!(target_pointer_width = "32") && !metadata_spec.scope.is_global() {
        let meta_chunk_addr = address_to_meta_chunk_addr(data_addr);
        let internal_addr = data_addr.as_usize() & CHUNK_MASK;
        let second_offset = internal_addr >> rshift;

        meta_chunk_addr + metadata_spec.offset + second_offset
    } else {
        unsafe {
            Address::from_usize(metadata_spec.offset + (data_addr.as_usize() >> rshift))
        }
    }
}

const fn addr_rshift(metadata_spec: SideMetadataSpec) -> i32 {
    ((constants::LOG_BITS_IN_BYTE as usize) + metadata_spec.log_min_obj_size
        - metadata_spec.log_num_of_bits) as i32
}

#[inline(always)]
pub(crate) const fn metadata_address_range_size(metadata_spec: SideMetadataSpec) -> usize {
    1usize << (LOG_ADDRESS_SPACE - addr_rshift(metadata_spec) as usize)
}

#[inline(always)]
pub(crate) fn meta_byte_lshift(metadata_spec: SideMetadataSpec, data_addr: Address) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits as i32;
    if bits_num_log >= 3 {
        return 0;
    }
    let rem_shift =
        constants::BITS_IN_WORD as i32 - ((constants::LOG_BITS_IN_BYTE as i32) - bits_num_log);
    ((((data_addr.as_usize() >> metadata_spec.log_min_obj_size) << rem_shift) >> rem_shift)
        << bits_num_log) as u8
}

#[inline(always)]
pub(crate) fn meta_byte_mask(metadata_spec: SideMetadataSpec) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits;
    ((1usize << (1usize << bits_num_log)) - 1) as u8
}
