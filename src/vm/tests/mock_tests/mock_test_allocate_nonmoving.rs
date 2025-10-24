// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;

#[test]
pub fn allocate_nonmoving() {
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
            let fixture = MutatorFixture::create_with_heapsize(MB);

            // Normal alloc
            let addr =
                memory_manager::alloc(fixture.mutator(), 16, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
            info!("Allocated default at: {:#x}", addr);

            // Non moving alloc
            let addr = memory_manager::alloc(
                fixture.mutator(),
                16,
                8,
                0,
                AllocationSemantics::NonMoving,
            );
            assert!(!addr.is_zero());
            info!("Allocated nonmoving at: {:#x}", addr);
        },
        no_cleanup,
    )
}
