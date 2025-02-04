use super::mock_test_prelude::*;

use crate::util::alloc::allocator::{AllocationOptions, OnAllocationFail};
use crate::AllocationSemantics;

/// This test will do alloc_no_gc in a loop, and evetually fill up the heap.
/// As alloc_no_gc will not trigger a GC, we expect to see a return value of zero, and no GC is triggered.
#[test]
pub fn allocate_no_gc_simple() {
    // 1MB heap
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(MB);

            let mut last_result = crate::util::Address::MAX;

            // Attempt allocation: allocate 1024 bytes. We should fill up the heap by 1024 allocations or fewer (some plans reserves more memory, such as semispace and generational GCs)
            // Run a few more times to test if we set/unset no_gc_on_fail properly.
            for _ in 0..1100 {
                last_result = memory_manager::alloc_with_options(
                    &mut fixture.mutator,
                    1024,
                    8,
                    0,
                    AllocationSemantics::Default,
                    AllocationOptions {
                        on_fail: OnAllocationFail::ReturnFailure,
                    },
                );
                if last_result.is_zero() {
                    read_mockvm(|mock| {
                        assert!(!mock.block_for_gc.is_called());
                    });
                    read_mockvm(|mock| {
                        assert!(!mock.out_of_memory.is_called());
                    });
                }
            }

            // The allocation should consume all the heap, and the last result should be zero (failure).
            assert!(last_result.is_zero());
        },
        no_cleanup,
    )
}
