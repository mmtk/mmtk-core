#[cfg(all(test, debug_assertions))]
mod tests {
    use atomic::Ordering;

    use crate::util::constants;
    use crate::util::heap::layout::vm_layout;
    use crate::util::heap::layout::vm_layout::vm_layout;
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
        let heap_start = vm_layout().heap_start;
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
                        .try_map_metadata_space(heap_start, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    gspec.assert_metadata_mapped(heap_start);
                    lspec.assert_metadata_mapped(heap_start);
                    gspec.assert_metadata_mapped(heap_start + constants::BYTES_IN_PAGE - 1);
                    lspec.assert_metadata_mapped(heap_start + constants::BYTES_IN_PAGE - 1);

                    metadata.ensure_unmap_metadata_space(heap_start, constants::BYTES_IN_PAGE);

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
                            heap_start + vm_layout::BYTES_IN_CHUNK,
                            vm_layout::BYTES_IN_CHUNK,
                        )
                        .is_ok());

                    gspec.assert_metadata_mapped(heap_start + vm_layout::BYTES_IN_CHUNK);
                    lspec.assert_metadata_mapped(heap_start + vm_layout::BYTES_IN_CHUNK);
                    gspec.assert_metadata_mapped(heap_start + vm_layout::BYTES_IN_CHUNK * 2 - 8);
                    lspec.assert_metadata_mapped(heap_start + vm_layout::BYTES_IN_CHUNK * 2 - 16);

                    metadata.ensure_unmap_metadata_space(
                        heap_start + vm_layout::BYTES_IN_CHUNK,
                        vm_layout::BYTES_IN_CHUNK,
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
                    let data_addr = vm_layout().heap_start;

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

                    let zero =
                        metadata_1_spec.fetch_add_atomic::<u16>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    let zero =
                        metadata_2_spec.fetch_add_atomic::<u8>(data_addr, 5, Ordering::SeqCst);
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
                    let data_addr = vm_layout().heap_start + (vm_layout::BYTES_IN_CHUNK << 1) * 2;

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

                    let zero =
                        metadata_1_spec.fetch_add_atomic::<u8>(data_addr, 2, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let two = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(two, 2);

                    let another_two =
                        metadata_1_spec.fetch_sub_atomic::<u8>(data_addr, 1, Ordering::SeqCst);
                    assert_eq!(another_two, 2);

                    let one = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(one, 1);

                    metadata_1_spec.store_atomic::<u8>(data_addr, 0, Ordering::SeqCst);

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
    fn test_side_metadata_atomic_fetch_and_or_2bits() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We need to do this because of the static NO_METADATA
                    // sanity::reset();
                    let data_addr =
                        vm_layout::vm_layout().heap_start + (vm_layout::BYTES_IN_CHUNK << 1);

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

                    let zero =
                        metadata_1_spec.fetch_or_atomic::<u8>(data_addr, 0b11, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let value_11 = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(value_11, 0b11);

                    let another_value_11 =
                        metadata_1_spec.fetch_and_atomic::<u8>(data_addr, 0b01, Ordering::SeqCst);
                    assert_eq!(another_value_11, 0b11);

                    let value_01 = metadata_1_spec.load_atomic::<u8>(data_addr, Ordering::SeqCst);
                    assert_eq!(value_01, 0b01);

                    metadata_1_spec.store_atomic::<u8>(data_addr, 0, Ordering::SeqCst);

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
                    let data_addr = vm_layout().heap_start + (vm_layout::BYTES_IN_CHUNK << 2);

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

                    let zero =
                        metadata_1_spec.fetch_add_atomic::<u16>(data_addr, 5, Ordering::SeqCst);
                    assert_eq!(zero, 0);

                    let five = metadata_1_spec.load_atomic::<u16>(data_addr, Ordering::SeqCst);
                    assert_eq!(five, 5);

                    let zero =
                        metadata_2_spec.fetch_add_atomic::<u8>(data_addr, 5, Ordering::SeqCst);
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

    #[test]
    fn test_side_metadata_bzero_by_bytes() {
        serial_test(|| {
            with_cleanup(
                || {
                    let data_addr = vm_layout::vm_layout().heap_start;

                    // 1 bit per 8 bytes
                    let spec = SideMetadataSpec {
                        name: "test spec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 0,
                        log_bytes_in_region: 3,
                    };
                    let region_size: usize = 1 << spec.log_bytes_in_region;

                    let metadata = SideMetadataContext {
                        global: vec![spec],
                        local: vec![],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    // First 9 regions
                    let regions = (0..9)
                        .map(|i| data_addr + (region_size * i))
                        .collect::<Vec<Address>>();
                    // Set metadata for the regions
                    regions
                        .iter()
                        .for_each(|addr| unsafe { spec.store::<u8>(*addr, 1) });
                    regions
                        .iter()
                        .for_each(|addr| assert!(unsafe { spec.load::<u8>(*addr) } == 1));

                    // bulk zero the 8 regions (1 bit for each, in total 1 byte)
                    spec.bzero_metadata(regions[0], region_size * 8);
                    // Check if the first 8 regions are set to 0
                    regions[0..8]
                        .iter()
                        .for_each(|addr| assert!(unsafe { spec.load::<u8>(*addr) } == 0));
                    // Check if the 9th region is still 1
                    assert!(unsafe { spec.load::<u8>(regions[8]) } == 1);
                },
                || {
                    sanity::reset();
                },
            )
        })
    }

    #[test]
    fn test_side_metadata_bzero_by_fraction_of_bytes() {
        serial_test(|| {
            with_cleanup(
                || {
                    let data_addr = vm_layout::vm_layout().heap_start;

                    // 1 bit per 8 bytes
                    let spec = SideMetadataSpec {
                        name: "test spec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits: 0,
                        log_bytes_in_region: 3,
                    };
                    let region_size: usize = 1 << spec.log_bytes_in_region;

                    let metadata = SideMetadataContext {
                        global: vec![spec],
                        local: vec![],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    assert!(metadata
                        .try_map_metadata_space(data_addr, constants::BYTES_IN_PAGE,)
                        .is_ok());

                    // First 9 regions
                    let regions = (0..9)
                        .map(|i| data_addr + (region_size * i))
                        .collect::<Vec<Address>>();
                    // Set metadata for the regions
                    regions
                        .iter()
                        .for_each(|addr| unsafe { spec.store::<u8>(*addr, 1) });
                    regions
                        .iter()
                        .for_each(|addr| assert!(unsafe { spec.load::<u8>(*addr) } == 1));

                    // bulk zero the first 4 regions (1 bit for each, in total 4 bits)
                    spec.bzero_metadata(regions[0], region_size * 4);
                    // Check if the first 4 regions are set to 0
                    regions[0..4]
                        .iter()
                        .for_each(|addr| assert!(unsafe { spec.load::<u8>(*addr) } == 0));
                    // Check if the rest regions is still 1
                    regions[4..9]
                        .iter()
                        .for_each(|addr| assert!(unsafe { spec.load::<u8>(*addr) } == 1));
                },
                || {
                    sanity::reset();
                },
            )
        })
    }

    #[test]
    fn test_side_metadata_zero_meta_bits() {
        let size = 4usize;
        let allocate_u32 = || -> Address {
            let ptr = unsafe {
                std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(size, 4).unwrap())
            };
            Address::from_mut_ptr(ptr)
        };
        let fill_1 = |addr: Address| unsafe {
            addr.store(u32::MAX);
        };

        let start = allocate_u32();
        let end = start + size;

        fill_1(start);
        // zero the word
        SideMetadataSpec::zero_meta_bits(start, 0, end, 0);
        assert_eq!(unsafe { start.load::<u32>() }, 0);

        fill_1(start);
        // zero first 2 bits
        SideMetadataSpec::zero_meta_bits(start, 0, start, 2);
        assert_eq!(unsafe { start.load::<u32>() }, 0xFFFF_FFFC); // ....1100

        fill_1(start);
        // zero last 2 bits
        SideMetadataSpec::zero_meta_bits(end - 1, 6, end, 0);
        assert_eq!(unsafe { start.load::<u32>() }, 0x3FFF_FFFF); // 0011....

        fill_1(start);
        // zero everything except first 2 bits and last 2 bits
        SideMetadataSpec::zero_meta_bits(start, 2, end - 1, 6);
        assert_eq!(unsafe { start.load::<u32>() }, 0xC000_0003); // 1100....0011
    }

    #[test]
    fn test_side_metadata_bcopy_metadata_contiguous() {
        serial_test(|| {
            with_cleanup(
                || {
                    let data_addr = vm_layout().heap_start;

                    let log_num_of_bits = 0;
                    let log_bytes_in_region = 3;
                    let num_regions = 0x400; // 1024
                    let bytes_per_region = 1 << log_bytes_in_region;
                    let total_size = num_regions * bytes_per_region; // 8192

                    let metadata_1_spec = SideMetadataSpec {
                        name: "metadata_1_spec",
                        is_global: true,
                        offset: SideMetadataOffset::addr(GLOBAL_SIDE_METADATA_BASE_ADDRESS),
                        log_num_of_bits,
                        log_bytes_in_region,
                    };

                    let metadata_2_spec = SideMetadataSpec {
                        name: "metadata_2_spec",
                        is_global: true,
                        offset: SideMetadataOffset::layout_after(&metadata_1_spec),
                        log_num_of_bits,
                        log_bytes_in_region,
                    };

                    // Currently global metadata are contiguous.
                    let metadata = SideMetadataContext {
                        global: vec![metadata_1_spec, metadata_2_spec],
                        local: vec![],
                    };

                    let mut metadata_sanity = SideMetadataSanity::new();
                    metadata_sanity.verify_metadata_context("NoPolicy", &metadata);

                    metadata
                        .try_map_metadata_space(data_addr, total_size)
                        .unwrap();

                    metadata_1_spec.bzero_metadata(data_addr, total_size);
                    metadata_2_spec.bzero_metadata(data_addr, total_size);

                    for i in 0..num_regions {
                        metadata_1_spec.store_atomic::<u8>(
                            data_addr + i * bytes_per_region,
                            (i % 2) as u8,
                            Ordering::Relaxed,
                        );
                    }

                    let test_copy_region = |begin: usize, end: usize| {
                        // Test copying whole bytes.
                        metadata_2_spec.bcopy_metadata_contiguous(
                            data_addr + begin * bytes_per_region,
                            (end - begin) * bytes_per_region,
                            &metadata_1_spec,
                        );

                        for i in 0..num_regions {
                            let bit = metadata_2_spec.load_atomic::<u8>(
                                data_addr + i * bytes_per_region,
                                Ordering::Relaxed,
                            );

                            let expected = if begin <= i && i < end {
                                (i % 2) as u8
                            } else {
                                0
                            };
                            assert_eq!(
                                bit, expected,
                                "Expected: {expected}, actual: {bit}, i: {i}, begin: {begin}, end: {end}"
                            );
                        }

                        metadata_2_spec.bzero_metadata(data_addr, total_size);
                    };

                    // Whole bytes
                    test_copy_region(0x100, 0x200);

                    // End unaligned
                    test_copy_region(0x18, 0xcc);

                    // Start unaligned
                    test_copy_region(0x263, 0x3f0);

                    // Start and end unaligned
                    test_copy_region(0x82, 0x1fd);

                    metadata_1_spec.bzero_metadata(data_addr, total_size);
                    metadata_2_spec.bzero_metadata(data_addr, total_size);

                    metadata_sanity.reset();
                },
                || {
                    sanity::reset();
                },
            );
        });
    }
}
