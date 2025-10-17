use super::mock_test_prelude::*;

use crate::util::alloc::allocator::AllocationOptions;
use crate::AllocationSemantics;

/// This test will allocate an object that is larger than the heap size. The call will fail by
/// returning null.
#[test]
pub fn allocate_no_gc_oom_on_acquire_no_oom_call() {
    // 1MB heap
    with_mockvm(
        default_setup,
        || {
            const KB: usize = 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(KB);

            // Attempt to allocate an object that is larger than the heap size.
            let addr = memory_manager::alloc_with_options(
                &mut fixture.mutator,
                1024 * 10,
                8,
                0,
                AllocationSemantics::Default,
                AllocationOptions {
                    at_safepoint: false,
                    allow_oom_call: false,
                    ..Default::default()
                },
            );
            // We should get zero.
            assert!(addr.is_zero());
            // block_for_gc and out_of_memory won't be called.
            read_mockvm(|mock| {
                assert!(!mock.block_for_gc.is_called());
            });
            read_mockvm(|mock| {
                assert!(!mock.out_of_memory.is_called());
            });
        },
        no_cleanup,
    )
}
