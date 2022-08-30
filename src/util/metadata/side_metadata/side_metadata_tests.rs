#[cfg(test)]
mod tests {
    use atomic::Ordering;

    use crate::util::constants;
    use crate::util::heap::layout::vm_layout_constants;
    use crate::util::metadata::side_metadata::SideMetadataContext;
    use crate::util::metadata::side_metadata::SideMetadataSpec;
    use crate::util::metadata::side_metadata::*;
    use crate::util::test_util::{serial_test, with_cleanup};
    use crate::util::Address;

    #[test]
    fn test_side_metadata_address_to_meta_address() {
        let mut gspec = SideMetadataSpec {
            name: "gspec",
            is_global: true,
            offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        #[cfg(target_pointer_width = "64")]
        let mut lspec = SideMetadataSpec {
            name: "lspec",
            is_global: false,
            offset: SideMetadataOffset::addr(LOCAL_SIDE_METADATA_BASE_ADDRESS),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        #[cfg(target_pointer_width = "32")]
        let mut lspec = SideMetadataSpec {
            name: "lspec",
            is_global: false,
            offset: SideMetadataOffset::rel(0),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(0) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(0) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(7) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(7) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(27) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS + 3usize
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(129) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS + 16usize
        );

        gspec.log_bytes_in_region = 2;
        lspec.log_bytes_in_region = 1;

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(0) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(0) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(32) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS + 1usize
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(32) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS + 2usize
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(316) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS + 9usize
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(316) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS + 19usize
        );

        gspec.log_num_of_bits = 1;
        lspec.log_num_of_bits = 3;

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(0) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(0) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(32) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS + 2usize
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(32) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS + 16usize
        );

        assert_eq!(
            address_to_meta_address(&gspec, unsafe { Address::from_usize(316) }),
            GLOBAL_SIDE_METADATA_BASE_ADDRESS + 19usize
        );
        assert_eq!(
            address_to_meta_address(&lspec, unsafe { Address::from_usize(318) }),
            LOCAL_SIDE_METADATA_BASE_ADDRESS + 159usize
        );
    }

    #[test]
    fn test_side_metadata_meta_byte_mask() {
        let mut spec = SideMetadataSpec {
            name: "test_spec",
            is_global: true,
            offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert_eq!(meta_byte_mask(&spec), 1);

        spec.log_num_of_bits = 1;
        assert_eq!(meta_byte_mask(&spec), 3);
        spec.log_num_of_bits = 2;
        assert_eq!(meta_byte_mask(&spec), 15);
        spec.log_num_of_bits = 3;
        assert_eq!(meta_byte_mask(&spec), 255);
    }

    #[test]
    fn test_side_metadata_meta_byte_lshift() {
        let mut spec = SideMetadataSpec {
            name: "test_spec",
            is_global: true,
            offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(0) }),
            0
        );
        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(5) }),
            5
        );
        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(15) }),
            7
        );

        spec.log_num_of_bits = 2;

        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(0) }),
            0
        );
        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(5) }),
            4
        );
        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(15) }),
            4
        );
        assert_eq!(
            meta_byte_lshift(&spec, unsafe { Address::from_usize(0x10010) }),
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
                        name: "gspec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 1,
                        log_bytes_in_region: 1,
                    };
                    #[cfg(target_pointer_width = "64")]
                    let mut lspec = SideMetadataSpec {
                        name: "lspec",
                        is_global: false,
                        offset: SideMetadataOffset::addr(LOCAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 1,
                        log_bytes_in_region: 1,
                    };
                    #[cfg(target_pointer_width = "32")]
                    let mut lspec = SideMetadataSpec {
                        name: "lspec",
                        is_global: false,
                        offset: SideMetadataOffset::rel(0),
                        log_num_of_bits: 1,
                        log_bytes_in_region: 1,
                    };

                    let metadata = SideMetadataContext {
                        global: vec![gspec],
                        local: vec![lspec],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(
                            vm_layout_constants::HEAP_START,
                            constants::BYTES_IN_PAGE,
                        )
                        .is_ok());

                    gspec.assert_metadata_mapped(vm_layout_constants::HEAP_START);
                    lspec.assert_metadata_mapped(vm_layout_constants::HEAP_START);
                    gspec.assert_metadata_mapped(
                        vm_layout_constants::HEAP_START + constants::BYTES_IN_PAGE - 1,
                    );
                    lspec.assert_metadata_mapped(
                        vm_layout_constants::HEAP_START + constants::BYTES_IN_PAGE - 1,
                    );

                    metadata.ensure_unmap_metadata_space(
                        vm_layout_constants::HEAP_START,
                        constants::BYTES_IN_PAGE,
                    );

                    gspec.log_bytes_in_region = 4;
                    gspec.log_num_of_bits = 4;
                    lspec.log_bytes_in_region = 4;
                    lspec.log_num_of_bits = 4;

                    metadata_sanity.reset();

                    let metadata = SideMetadataContext {
                        global: vec![gspec],
                        local: vec![lspec],
                    };

                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);
                    metadata_sanity.reset();

                    assert!(metadata
                        .try_map_metadata_space(
                            vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                            vm_layout_constants::BYTES_IN_CHUNK,
                        )
                        .is_ok());

                    gspec.assert_metadata_mapped(
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                    );
                    lspec.assert_metadata_mapped(
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK,
                    );
                    gspec.assert_metadata_mapped(
                        vm_layout_constants::HEAP_START + vm_layout_constants::BYTES_IN_CHUNK * 2
                            - 8,
                    );
                    lspec.assert_metadata_mapped(
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
                        name: "metadata_1_spec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 4,
                        log_bytes_in_region: 6,
                    };

                    let metadata_2_spec = SideMetadataSpec {
                        name: "metadata_2_spec",
                        is_global: true,
                        offset: SideMetadataOffset::layout_after(&metadata_1_spec),
                        log_num_of_bits: 3,
                        log_bytes_in_region: 7,
                    };

                    let metadata = SideMetadataContext {
                        global: vec![metadata_1_spec, metadata_2_spec],
                        local: vec![],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = metadata_1_spec.fetch_add_atomic::<u16>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    let zero = metadata_2_spec.fetch_add_atomic::<u8>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_2_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    let another_five =
                        metadata_1_spec.fetch_sub_atomic::<u16>(data_addr, 2, Ordering::SeqCst);
                    assert_eq!(another_five, 5);

                    let three = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(three, 3);

                    let another_five =
                        metadata_2_spec.fetch_sub_atomic::<u8>(data_addr, 2, Ordering::SeqCst);
                    assert_eq!(another_five, 5);

                    let three = metadata_2_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(three, 3);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);
                    metadata_sanity.reset();
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
                        name: "metadata_1_spec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 1,
                        log_bytes_in_region: constants::LOG_BYTES_IN_WORD as usize,
                    };

                    let metadata = SideMetadataContext {
                        global: vec![metadata_1_spec],
                        local: vec![],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = metadata_1_spec.fetch_add_atomic::<u8>(data_addr, 2, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let two = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(two, 2);

                    let another_two =
                        metadata_1_spec.fetch_sub_atomic::<u8>(data_addr, 1, Ordering::SeqCst);
                    assert_eq!(another_two, 2);

                    let one = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(one, 1);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);

                    metadata_sanity.reset();
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
                        name: "metadata_1_spec",
                        is_global: false,
                        offset: SideMetadataOffset::addr(LOCAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 4,
                        log_bytes_in_region: 9,
                    };
                    #[cfg(target_pointer_width = "64")]
                    let metadata_2_spec = SideMetadataSpec {
                        name: "metadata_2_spec",
                        is_global: false,
                        offset: SideMetadataOffset::layout_after(&metadata_1_spec),
                        log_num_of_bits: 3,
                        log_bytes_in_region: 7,
                    };

                    #[cfg(target_pointer_width = "32")]
                    let metadata_1_spec = SideMetadataSpec {
                        name: "metadata_1_spec",
                        is_global: false,
                        offset: SideMetadataOffset::rel(0),
                        log_num_of_bits: 4,
                        log_bytes_in_region: 9,
                    };
                    #[cfg(target_pointer_width = "32")]
                    let metadata_2_spec = SideMetadataSpec {
                        name: "metadata_2_spec",
                        is_global: false,
                        offset: SideMetadataOffset::layout_after(&metadata_1_spec),
                        log_num_of_bits: 3,
                        log_bytes_in_region: 7,
                    };

                    let metadata = SideMetadataContext {
                        global: vec![],
                        local: vec![metadata_1_spec, metadata_2_spec],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    let zero = metadata_1_spec.fetch_add_atomic::<u16>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    let zero = metadata_2_spec.fetch_add_atomic::<u8>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_2_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    metadata_2_spec.bzero_metadata(data_addr, constants::BYTES_IN_PAGE);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);
                    let five = metadata_2_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 0);

                    metadata_1_spec.bzero_metadata(data_addr, constants::BYTES_IN_PAGE);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 0);
                    let five = metadata_2_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 0);

                    metadata.ensure_unmap_metadata_space(data_addr, constants::BYTES_IN_PAGE);

                    metadata_sanity.reset();
                },
                || {
                    sanity::reset();
                },
            );
        });
    }
}
