// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Allocation churn through real (stop-the-world) GCs: allocate far more than the heap size, so
//! that the plan collects many times. MockVM has no roots, so every GC frees everything, and the
//! sweep must return memory for allocation to continue. Each new object must be zeroed.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;

#[test]
pub fn gc_churn() {
    with_mockvm(
        default_setup,
        || {
            const HEAP: usize = 8 << 20;
            let fixture = MutatorFixture::create_with_heapsize(HEAP);
            let Some(space) = span_space(fixture.mmtk()) else {
                eprintln!("No SpanSpace in this plan: test skipped");
                return;
            };
            let mutator = fixture.mutator();
            let sizes = [16usize, 40, 96, 333, 1024, 2900, 7000, 15000, 32000];

            // A fixed total: 16x the heap.
            let mut total = 0;
            let mut i = 0;
            while total < 16 * HEAP {
                let size = sizes[i % sizes.len()];
                let obj = alloc_object(mutator, size);
                assert_zeroed(obj, size);
                fill(obj, size, 0xEE);
                total += size;
                i += 1;
            }
            // GCs happened and swept: the space did not grow beyond the heap.
            assert!(space.pages_in_use() * crate::policy::spanspace::BYTES_IN_SPAN_PAGE <= HEAP);
        },
        no_cleanup,
    )
}
