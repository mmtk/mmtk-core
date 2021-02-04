use super::constants::*;
use super::SideMetadataSpec;
use crate::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;
use crate::util::{constants, Address};

#[inline(always)]
pub(crate) fn address_to_meta_chunk_addr(data_addr: Address) -> Address {
    SIDE_METADATA_BASE_ADDRESS
        + ((data_addr.as_usize() & !CHUNK_MASK) >> SIDE_METADATA_WORST_CASE_RATIO_LOG)
}

#[inline(always)]
pub(crate) fn address_to_meta_address(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
) -> Address {
    let bits_num_log = metadata_spec.log_num_of_bits as i32;
    let log_min_obj_size = metadata_spec.log_min_obj_size as i32;
    let first_offset = if metadata_spec.scope.is_global() {
        metadata_spec.offset
    } else {
        metadata_spec.offset + POLICY_SIDE_METADATA_OFFSET
    };

    let meta_chunk_addr = address_to_meta_chunk_addr(data_addr);
    let internal_addr = data_addr.as_usize() & CHUNK_MASK;
    let rshift = (constants::LOG_BITS_IN_WORD as i32) + log_min_obj_size - bits_num_log;
    debug_assert!(rshift >= 0);

    let second_offset = internal_addr >> rshift;

    meta_chunk_addr + first_offset + second_offset
}

#[inline(always)]
pub(crate) const fn meta_bytes_per_chunk(log_min_obj_size: usize, log_num_of_bits: usize) -> usize {
    1usize
        << (LOG_BYTES_IN_CHUNK - constants::LOG_BITS_IN_WORD - log_min_obj_size + log_num_of_bits)
}

#[inline(always)]
pub(super) fn meta_byte_lshift(metadata_spec: SideMetadataSpec, data_addr: Address) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits;
    let rem_shift =
        constants::LOG_BITS_IN_WORD - ((constants::LOG_BITS_IN_BYTE as usize) - bits_num_log);
    ((((data_addr.as_usize() >> metadata_spec.log_min_obj_size) << rem_shift) >> rem_shift)
        << bits_num_log) as u8
}

#[inline(always)]
pub(super) fn meta_byte_mask(metadata_spec: SideMetadataSpec) -> u8 {
    let bits_num_log = metadata_spec.log_num_of_bits;
    ((1usize << (1usize << bits_num_log)) - 1) as u8
}

#[cfg(test)]
mod tests {
    use super::address_to_meta_address;
    use crate::util::side_metadata::constants::*;
    use crate::util::side_metadata::global::*;
    use crate::util::side_metadata::helpers::*;
    use crate::util::Address;

    #[test]
    fn test_side_metadata_address_to_meta_address() {
        let mut gspec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };
        let mut lspec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(0) }).as_usize(),
            SIDE_METADATA_BASE_ADDRESS.as_usize()
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(0) }).as_usize(),
            (SIDE_METADATA_BASE_ADDRESS + POLICY_SIDE_METADATA_OFFSET).as_usize()
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            SIDE_METADATA_BASE_ADDRESS.as_usize() + (meta_bytes_per_chunk(0, 0) >> 1)
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            (SIDE_METADATA_BASE_ADDRESS + POLICY_SIDE_METADATA_OFFSET).as_usize()
                + (meta_bytes_per_chunk(0, 0) >> 1)
        );

        gspec.log_min_obj_size = 2;
        lspec.log_min_obj_size = 1;

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            SIDE_METADATA_BASE_ADDRESS.as_usize() + (meta_bytes_per_chunk(2, 0) >> 1)
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            (SIDE_METADATA_BASE_ADDRESS + POLICY_SIDE_METADATA_OFFSET).as_usize()
                + (meta_bytes_per_chunk(1, 0) >> 1)
        );

        gspec.log_num_of_bits = 1;
        lspec.log_num_of_bits = 3;

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            SIDE_METADATA_BASE_ADDRESS.as_usize() + (meta_bytes_per_chunk(2, 1) >> 1)
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(BYTES_IN_CHUNK >> 1) })
                .as_usize(),
            (SIDE_METADATA_BASE_ADDRESS + POLICY_SIDE_METADATA_OFFSET).as_usize()
                + (meta_bytes_per_chunk(1, 3) >> 1)
        );
    }

    #[test]
    fn test_side_metadata_meta_byte_mask() {
        let mut spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };

        assert_eq!(meta_byte_mask(spec), 1);

        spec.log_num_of_bits = 1;
        assert_eq!(meta_byte_mask(spec), 3);
        spec.log_num_of_bits = 2;
        assert_eq!(meta_byte_mask(spec), 15);
        spec.log_num_of_bits = 3;
        assert_eq!(meta_byte_mask(spec), 255);
    }

    #[test]
    fn test_side_metadata_meta_byte_lshift() {
        let mut spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };

        assert_eq!(meta_byte_lshift(spec, unsafe { Address::from_usize(0) }), 0);
        assert_eq!(meta_byte_lshift(spec, unsafe { Address::from_usize(5) }), 5);
        assert_eq!(
            meta_byte_lshift(spec, unsafe { Address::from_usize(15) }),
            7
        );

        spec.log_num_of_bits = 2;

        assert_eq!(meta_byte_lshift(spec, unsafe { Address::from_usize(0) }), 0);
        assert_eq!(meta_byte_lshift(spec, unsafe { Address::from_usize(5) }), 4);
        assert_eq!(
            meta_byte_lshift(spec, unsafe { Address::from_usize(15) }),
            4
        );
        assert_eq!(
            meta_byte_lshift(spec, unsafe { Address::from_usize(0x10010) }),
            0
        );
    }

    #[test]
    fn test_side_metadata_meta_bytes_per_chunk() {
        let ch_sz = BYTES_IN_CHUNK;
        let bw = constants::BITS_IN_WORD;
        assert_eq!(meta_bytes_per_chunk(0, 0), ch_sz / bw);
        assert_eq!(meta_bytes_per_chunk(3, 0), (ch_sz / bw) >> 3);
        assert_eq!(meta_bytes_per_chunk(0, 3), (ch_sz / bw) << 3);
    }
}
