// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::*;
use crate::vm::ObjectModel;
use crate::AllocationSemantics;

#[test]
pub fn interior_poiner_before_in_object_addr() {
    const MB: usize = 1024 * 1024;
    const OBJECT_SIZE: usize = 32;

    // The object layout is like this: 0x1000 object start, 0x1008 in-object addr, 0x1010 object reference, 0x1020 object end.
    // 16 bytes header
    const OBJECT_REF_OFFSET: usize = 16;
    // in-object addr is 8 bytes before object ref
    const IN_OBJECT_ADDR_OFFSET: usize = 8;
    with_mockvm(
        || -> MockVM {
            MockVM {
                ref_to_object_start: MockMethod::new_fixed(Box::new(|object| {
                    object.to_raw_address().sub(OBJECT_REF_OFFSET)
                })),
                ref_to_address: MockMethod::new_fixed(Box::new(|object| {
                    object.to_raw_address().sub(IN_OBJECT_ADDR_OFFSET)
                })),
                address_to_ref: MockMethod::new_fixed(Box::new(|addr| {
                    ObjectReference::from_raw_address(addr.add(IN_OBJECT_ADDR_OFFSET)).unwrap()
                })),
                get_object_size: MockMethod::new_fixed(Box::new(|_| OBJECT_SIZE)),
                ..MockVM::default()
            }
        },
        || {
            let mut fixture = MutatorFixture::create_with_heapsize(10 * MB);

            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                OBJECT_SIZE,
                8,
                0,
                AllocationSemantics::Default,
            );
            assert!(!addr.is_zero());

            let obj = ObjectReference::from_raw_address(addr + OBJECT_REF_OFFSET).unwrap();
            println!(
                "start = {}, end = {}, obj = {}, in-obj addr = {}",
                addr,
                addr + OBJECT_SIZE,
                obj,
                obj.to_address::<MockVM>()
            );
            memory_manager::post_alloc(
                &mut fixture.mutator,
                obj,
                OBJECT_SIZE,
                AllocationSemantics::Default,
            );

            // Forge a pointer that points before the in-object address. MMTk should not consider it as internal pointer, and it is undefined
            // behavior to call `find_object_from_internal_pointer`

            let before_in_object_addr = addr;
            assert!(before_in_object_addr < MockVM::ref_to_address(obj));

            println!("Check {:?}", before_in_object_addr);
            let base_ref = crate::memory_manager::find_object_from_internal_pointer::<MockVM>(
                before_in_object_addr,
                usize::MAX,
            );
            println!("base_ref {:?}", base_ref);
            // It is undefined behavior.
            // For PageProtect which use LOS, we can find the object reference. For everything else, we cannot find the object refernce.
            // This is not a guarantee though. This test simply asserts the current behavior.
            if *fixture.mmtk().get_options().plan == crate::util::options::PlanSelector::PageProtect
            {
                assert!(base_ref.is_some());
                assert_eq!(base_ref.unwrap(), obj);
            } else {
                assert!(base_ref.is_none());
            }
        },
        no_cleanup,
    )
}
