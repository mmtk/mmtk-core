// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;

#[test]
pub fn simple_gc() {
    with_mockvm(
        default_setup,
        || {
            // 1MB heap
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(MB);

            // Normal alloc
            let addr =
                memory_manager::alloc(fixture.mutator(), 16, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
            info!("Allocated default at: {:#x}", addr);

            memory_manager::handle_user_collection_request(&fixture.mmtk(), fixture.mutator_tls());
        },
        no_cleanup,
    )
}
