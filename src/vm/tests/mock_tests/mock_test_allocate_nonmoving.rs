// GITHUB-CI: MMTK_PLAN=all

use lazy_static::lazy_static;

use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;

#[test]
pub fn allocate_alignment() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                is_collection_enabled: MockMethod::new_fixed(Box::new(|_| false)),
                ..MockVM::default()
            }
        },
        || {
            // 1MB heap
            const MB: usize = 1024 * 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(MB);

            // Normal alloc
            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                16,
                8,
                0,
                AllocationSemantics::Default,
            );
            assert!(!addr.is_zero());

            // Non moving alloc
            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                16,
                8,
                0,
                AllocationSemantics::NonMoving,
            );
            assert!(!addr.is_zero());
        },
        no_cleanup,
    )
}
