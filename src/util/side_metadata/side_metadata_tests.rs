#[cfg(test)]
mod tests {
    use crate::util::constants;
    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::side_metadata::*;
    use crate::util::test_util::{serial_test, with_cleanup};
    use crate::util::Address;

    #[test]
    fn test_side_metadata_address_to_meta_address() {
        let mut gspec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };
        #[cfg(target_pointer_width = "64")]
        let mut lspec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };

        #[cfg(target_pointer_width = "32")]
        let mut lspec = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: 0,
            log_num_of_bits: 0,
            log_min_obj_size: 0,
        };

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(0) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(0) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(7) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(7) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(27) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 3
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(129) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 16
        );

        gspec.log_min_obj_size = 2;
        lspec.log_min_obj_size = 1;

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(0) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(0) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(32) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 1
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(32) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 2
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(316) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 9
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(316) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 19
        );

        gspec.log_num_of_bits = 1;
        lspec.log_num_of_bits = 3;

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(0) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(0) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(32) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 2
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(32) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 16
        );

        assert_eq!(
            address_to_meta_address(gspec, unsafe { Address::from_usize(316) }).as_usize(),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 19
        );
        assert_eq!(
            address_to_meta_address(lspec, unsafe { Address::from_usize(318) }).as_usize(),
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + 159
        );
    }

    #[test]
    fn test_side_metadata_meta_byte_mask() {
        let mut spec = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
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
            offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
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
    fn test_side_metadata_try_mmap_metadata() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We need to do this because of the static NO_METADATA
                    // sanity::reset();
                    let mut gspec = SideMetadataSpec {
                        scope: SideMetadataScope::Global,
                        offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
                        log_num_of_bits: 0,
                        log_min_obj_size: 0,
                    };
                    #[cfg(target_pointer_width = "64")]
                    let mut lspec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
                        log_num_of_bits: 1,
                        log_min_obj_size: 1,
                    };
                    #[cfg(target_pointer_width = "32")]
                    let mut lspec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: 0,
                        log_num_of_bits: 1,
                        log_min_obj_size: 1,
                    };

                    let metadata = SideMetadata::new(
                        "NoPolicy",
                        SideMetadataContext {
                            global: vec![gspec],
                            local: vec![lspec],
                        },
                    );

                    assert!(metadata
                        .try_map_metadata_space(
                            vm_layout_constants::HEAP_START,
                            constants::BYTES_IN_PAGE,
                        )
                        .is_ok());

                    ensure_metadata_is_mapped(gspec, vm_layout_constants::HEAP_START);
                    ensure_metadata_is_mapped(lspec, vm_layout_constants::HEAP_START);
                    ensure_metadata_is_mapped(
                        gspec,
                        vm_layout_constants::HEAP_START + constants::BYTES_IN_PAGE - 1,
                    );
                    ensure_metadata_is_mapped(
                        lspec,
                        vm_layout_constants::HEAP_START + constants::BYTES_IN_PAGE - 1,
                    );

                    metadata.ensure_unmap_metadata_space(
                        vm_layout_constants::HEAP_START,
                        constants::BYTES_IN_PAGE,
                    );

                    gspec.log_min_obj_size = 3;
                    gspec.log_num_of_bits = 2;
                    lspec.log_min_obj_size = 4;
                    lspec.log_num_of_bits = 2;

                    sanity::reset();

                    let metadata = SideMetadata::new(
                        "NoPolicy",
                        SideMetadataContext {
                            global: vec![gspec],
                            local: vec![lspec],
                        },
                    );

                    assert!(metadata
                        .try_map_metadata_space(
                            vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                            vm_layout_constants::BYTES_IN_CHUNK,
                        )
                        .is_ok());

                    ensure_metadata_is_mapped(
                        gspec,
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                    );
                    ensure_metadata_is_mapped(
                        lspec,
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                    );
                    ensure_metadata_is_mapped(
                        gspec,
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK * 2
                            - 8,
                    );
                    ensure_metadata_is_mapped(
                        lspec,
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK * 2
                            - 16,
                    );

                    metadata.ensure_unmap_metadata_space(
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                        vm_layout_constants::BYTES_IN_CHUNK,
                    );
                },
                || {
                    sanity::reset();
                },
            );
        })
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_ge8bits() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We need to do this because of the static NO_METADATA
                    // sanity::reset();
                    let data_addr = vm_layout_constants::HEAP_START;

                    let metadata_1_spec = SideMetadataSpec {
                        scope: SideMetadataScope::Global,
                        offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
                        log_num_of_bits: 4,
                        log_min_obj_size: 6,
                    };

                    let metadata_2_spec = SideMetadataSpec {
                        scope: SideMetadataScope::Global,
                        offset: metadata_1_spec.offset
                            + metadata_address_range_size(metadata_1_spec),
                        log_num_of_bits: 3,
                        log_min_obj_size: 7,
                    };

                    let metadata = SideMetadata::new(
                        "NoPolicy",
                        SideMetadataContext {
                            global: vec![metadata_1_spec, metadata_2_spec],
                            local: vec![],
                        },
                    );

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = fetch_add_atomic(metadata_1_spec, data_addr, 5);
                    assert_eq!(zero, 0);

                    let five = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(five, 5);

                    let zero = fetch_add_atomic(metadata_2_spec, data_addr, 5);
                    assert_eq!(zero, 0);

                    let five = load_atomic(metadata_2_spec, data_addr);
                    assert_eq!(five, 5);

                    let another_five = fetch_sub_atomic(metadata_1_spec, data_addr, 2);
                    assert_eq!(another_five, 5);

                    let three = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(three, 3);

                    let another_five = fetch_sub_atomic(metadata_2_spec, data_addr, 2);
                    assert_eq!(another_five, 5);

                    let three = load_atomic(metadata_2_spec, data_addr);
                    assert_eq!(three, 3);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);
                },
                || {
                    sanity::reset();
                },
            );
        });
    }

    #[test]
    fn test_side_metadata_atomic_fetch_add_sub_2bits() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We need to do this because of the static NO_METADATA
                    // sanity::reset();
                    let data_addr = vm_layout_constants::HEAP_START
                        + (vm_layout_constants::BYTES_IN_CHUNK << 1);

                    let metadata_1_spec = SideMetadataSpec {
                        scope: SideMetadataScope::Global,
                        offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
                        log_num_of_bits: 1,
                        log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
                    };

                    let metadata = SideMetadata::new(
                        "NoPolicy",
                        SideMetadataContext {
                            global: vec![metadata_1_spec],
                            local: vec![],
                        },
                    );

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = fetch_add_atomic(metadata_1_spec, data_addr, 2);
                    assert_eq!(zero, 0);

                    let two = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(two, 2);

                    let another_two = fetch_sub_atomic(metadata_1_spec, data_addr, 1);
                    assert_eq!(another_two, 2);

                    let one = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(one, 1);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);
                },
                || {
                    sanity::reset();
                },
            );
        });
    }

    #[test]
    fn test_side_metadata_bzero_metadata() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We need to do this because of the static NO_METADATA
                    // sanity::reset();
                    let data_addr = vm_layout_constants::HEAP_START
                        + (vm_layout_constants::BYTES_IN_CHUNK << 2);

                    #[cfg(target_pointer_width = "64")]
                    let metadata_1_spec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
                        log_num_of_bits: 4,
                        log_min_obj_size: 9,
                    };
                    #[cfg(target_pointer_width = "64")]
                    let metadata_2_spec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: metadata_1_spec.offset
                            + metadata_address_range_size(metadata_1_spec),
                        log_num_of_bits: 3,
                        log_min_obj_size: 7,
                    };

                    #[cfg(target_pointer_width = "32")]
                    let metadata_1_spec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: 0,
                        log_num_of_bits: 4,
                        log_min_obj_size: 9,
                    };
                    #[cfg(target_pointer_width = "32")]
                    let metadata_2_spec = SideMetadataSpec {
                        scope: SideMetadataScope::PolicySpecific,
                        offset: metadata_1_spec.offset
                            + meta_bytes_per_chunk(
                                metadata_1_spec.log_min_obj_size,
                                metadata_1_spec.log_num_of_bits,
                            ),
                        log_num_of_bits: 3,
                        log_min_obj_size: 7,
                    };

                    let metadata = SideMetadata::new(
                        "NoPolicy",
                        SideMetadataContext {
                            global: vec![],
                            local: vec![metadata_1_spec, metadata_2_spec],
                        },
                    );

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = fetch_add_atomic(metadata_1_spec, data_addr, 5);
                    assert_eq!(zero, 0);

                    let five = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(five, 5);

                    let zero = fetch_add_atomic(metadata_2_spec, data_addr, 5);
                    assert_eq!(zero, 0);

                    let five = load_atomic(metadata_2_spec, data_addr);
                    assert_eq!(five, 5);

                    bzero_metadata(metadata_2_spec, data_addr, constants::BYTES_IN_PAGE);

                    let five = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(five, 5);
                    let five = load_atomic(metadata_2_spec, data_addr);
                    assert_eq!(five, 0);

                    bzero_metadata(metadata_1_spec, data_addr, constants::BYTES_IN_PAGE);

                    let five = load_atomic(metadata_1_spec, data_addr);
                    assert_eq!(five, 0);
                    let five = load_atomic(metadata_2_spec, data_addr);
                    assert_eq!(five, 0);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);
                },
                || {
                    sanity::reset();
                },
            );
        });
    }
}
