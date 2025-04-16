// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::AllocationSemantics;

#[test]
pub fn interior_pointer_before_object_ref() {
    const MB: usize = 1024 * 1024;
    const OBJECT_SIZE: usize = 16;
    with_mockvm(
        || -> MockVM {
            MockVM {
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

            let obj = MockVM::object_start_to_ref(addr);
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

            // Forge a pointer that points before the object reference, but after in-object address. MMTk should still find the base reference properly.

            let before_obj_ref = addr;
            assert!(before_obj_ref < obj.to_raw_address());
            assert!(before_obj_ref >= obj.to_address::<MockVM>());

            println!("Check {:?}", before_obj_ref);
            let base_ref = crate::memory_manager::find_object_from_internal_pointer::<MockVM>(
                before_obj_ref,
                usize::MAX,
            );
            println!("base_ref {:?}", base_ref);
            assert!(base_ref.is_some());
            assert_eq!(base_ref.unwrap(), obj);
        },
        no_cleanup,
    )
}
