use super::mock_test_prelude::*;

use crate::AllocationSemantics;

#[test]
pub fn issue139_alloc_non_multiple_of_min_alignment() {
    with_mockvm(
        default_setup,
        || {
            let mut fixture = MutatorFixture::create();

            // Allocate 6 bytes with 8 bytes ailgnment required
            let addr =
                memory_manager::alloc(&mut fixture.mutator, 14, 8, 0, AllocationSemantics::Default);
            assert!(addr.is_aligned_to(8));
            // After the allocation, the cursor is not MIN_ALIGNMENT aligned. If we have the assertion in the next allocation to check if the cursor is aligned to MIN_ALIGNMENT, it fails.
            // We have to remove that assertion.
            let addr2 =
                memory_manager::alloc(&mut fixture.mutator, 14, 8, 0, AllocationSemantics::Default);
            assert!(addr2.is_aligned_to(8));
        },
        no_cleanup,
    )
}
