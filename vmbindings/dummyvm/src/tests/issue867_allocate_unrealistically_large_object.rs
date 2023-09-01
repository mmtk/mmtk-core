// GITHUB-CI: MMTK_PLAN=all

use crate::api;
use crate::test_fixtures::{MutatorFixture, SerialFixture};
use mmtk::plan::AllocationSemantics;

lazy_static! {
    static ref MUTATOR: SerialFixture<MutatorFixture> = SerialFixture::new();
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_max_size_object() {
    let (size, align) = (usize::MAX, 8);

    MUTATOR.with_fixture_expect_benign_panic(|fixture| {
        api::mmtk_alloc(
            fixture.mutator,
            size,
            align,
            0,
            AllocationSemantics::Default,
        );
    })
}

#[test]
// This test panics with 'attempt to add with overflow', as we do computation with the size
// in the fastpath. I don't think we want to do any extra check in the fastpath. There is
// nothing we can do with it without sacrificing performance.
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
#[ignore]
pub fn allocate_max_size_object_after_succeed() {
    MUTATOR.with_fixture_expect_benign_panic(|fixture| {
        // Allocate something so we have a thread local allocation buffer
        api::mmtk_alloc(fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        // Allocate an unrealistically large object
        api::mmtk_alloc(
            fixture.mutator,
            usize::MAX,
            8,
            0,
            AllocationSemantics::Default,
        );
    })
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_unrealistically_large_object() {
    const CHUNK: usize = 4 * 1024 * 1024; // 4MB
                                          // Leave some room, so we won't have arithmetic overflow when we compute size and do alignment.
    let (size, align) = (
        mmtk::util::conversions::raw_align_down(usize::MAX - CHUNK, 4096),
        8,
    );

    MUTATOR.with_fixture_expect_benign_panic(|fixture| {
        api::mmtk_alloc(
            fixture.mutator,
            size,
            align,
            0,
            AllocationSemantics::Default,
        );
    })
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_more_than_heap_size() {
    // The heap has 1 MB. Allocating with 2MB will cause OOM.
    MUTATOR.with_fixture_expect_benign_panic(|fixture| {
        api::mmtk_alloc(
            fixture.mutator,
            2 * 1024 * 1024,
            8,
            0,
            AllocationSemantics::Default,
        );
    })
}
