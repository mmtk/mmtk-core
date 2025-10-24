// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

use crate::util::alloc::allocator::AllocationOptions;
use crate::AllocationSemantics;

/// This test will do alloc_with_options in a loop, and evetually fill up the heap.
/// As we require alloc_with_options to over commit, we expect to see valid return values, and no GC is triggered.
#[test]
pub fn allocate_overcommit() {
    // 1MB heap
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(MB);

            if *fixture.mmtk().get_plan().options().plan == crate::util::options::PlanSelector::NoGC {
                // Overcommit still triggers GC. For NoGC plan, triggering GC causes panic.
                return;
            }

            let mut last_result = crate::util::Address::MAX;

            // Attempt allocation: allocate 1024 bytes. We should fill up the heap by 1024 allocations or fewer (some plans reserves more memory, such as semispace and generational GCs)
            // Run a few more times to test if we set/unset no_gc_on_fail properly.
            for _ in 0..1100 {
                last_result = memory_manager::alloc_with_options(
                    fixture.mutator(),
                    1024,
                    8,
                    0,
                    AllocationSemantics::Default,
                    AllocationOptions {
                        allow_overcommit: true,
                        ..Default::default()
                    },
                );
                assert!(!last_result.is_zero());
                read_mockvm(|mock| {
                    assert!(!mock.block_for_gc.is_called());
                });
                read_mockvm(|mock| {
                    assert!(!mock.out_of_memory.is_called());
                });
            }

            // The allocation should consume all the heap, but we allow over commit and the last result should be not zero (failure).
            assert!(!last_result.is_zero());
        },
        no_cleanup,
    )
}
