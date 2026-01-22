// GITHUB-CI: MMTK_PLAN=Immix,GenImmix,StickyImmix,MarkSweep,MarkCompact
// GITHUB-CI: FEATURES=vo_bit

// Only test this with plans that use LOS. NoGC does not use large object space.

use super::mock_test_prelude::*;

use crate::AllocationSemantics;

#[test]
pub fn interior_pointer_in_large_object_same_page() {
    const MB: usize = 1024 * 1024;
    // Usually we will not see allocation in large object space that is smaller than a page.
    // But let's allow it for the page protect plan.
    const OBJECT_SIZE: usize = 256;
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
                AllocationSemantics::Los,
            );
            assert!(!addr.is_zero());

            let obj = MockVM::object_start_to_ref(addr);
            println!(
                "start = {}, end = {}, obj = {}",
                addr,
                addr + OBJECT_SIZE,
                obj,
            );

            memory_manager::post_alloc(
                &mut fixture.mutator,
                obj,
                OBJECT_SIZE,
                AllocationSemantics::Los,
            );

            let ptr = obj.to_raw_address();
            let base_ref =
                crate::memory_manager::find_object_from_internal_pointer(ptr, OBJECT_SIZE);
            println!("{:?}", base_ref);
            assert!(base_ref.is_some());
            assert_eq!(base_ref.unwrap(), obj);

            let ptr = obj.to_raw_address() + OBJECT_SIZE / 2;
            let base_ref =
                crate::memory_manager::find_object_from_internal_pointer(ptr, OBJECT_SIZE);
            assert!(base_ref.is_some());
            assert_eq!(base_ref.unwrap(), obj);

            let ptr = obj.to_raw_address() + OBJECT_SIZE;
            let base_ref =
                crate::memory_manager::find_object_from_internal_pointer(ptr, OBJECT_SIZE);
            assert!(base_ref.is_none());
        },
        no_cleanup,
    )
}
