use super::mock_test_prelude::*;

use crate::AllocationSemantics;

/// This test will allocate an object that is larger than the heap size. This should
/// not be an infinite loop. It should call `Collection::out_of_memory` and return null.
#[test]
pub fn allocate_no_infinite_loop_if_throw_oom_returns() {
    // 1MB heap
    with_mockvm(
        || -> MockVM {
            MockVM {
                out_of_memory: MockMethod::new_default(),
                ..MockVM::default()
            }
        },
        || {
            const KB: usize = 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(KB);

            // Attempt to allocate an object that is larger than the heap size.
            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                1024 * 1024,
                8,
                0,
                AllocationSemantics::Default,
            );
            // We should get zero.
            assert!(addr.is_zero());
            // out_of_memory should be called.
            read_mockvm(|mock| {
                assert!(mock.out_of_memory.is_called());
            });
        },
        no_cleanup,
    )
}
