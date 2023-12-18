// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

use crate::plan::AllocationSemantics;
use crate::util::Address;
use crate::Mutator;
use crate::MMTK;

lazy_static! {
    static ref MUTATOR: Fixture<MutatorFixture> = Fixture::new();
}

// Checks if the allocation should be an LOS allocation.
fn alloc_default_or_large(
    mmtk: &MMTK<MockVM>,
    mutator: &mut Mutator<MockVM>,
    size: usize,
    align: usize,
    offset: usize,
    semantic: AllocationSemantics,
) -> Address {
    let max_non_los_size = mmtk
        .get_plan()
        .constraints()
        .max_non_los_default_alloc_bytes;
    if size >= max_non_los_size {
        memory_manager::alloc(mutator, size, align, offset, AllocationSemantics::Los)
    } else {
        memory_manager::alloc(mutator, size, align, offset, semantic)
    }
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_max_size_object() {
    with_mockvm(
        default_setup,
        || {
            let (size, align) = (usize::MAX, 8);

            MUTATOR.with_fixture_mut(|fixture| {
                alloc_default_or_large(
                    fixture.mmtk(),
                    &mut fixture.mutator,
                    size,
                    align,
                    0,
                    AllocationSemantics::Default,
                );
            })
        },
        no_cleanup,
    )
}

#[test]
// This test panics with 'attempt to add with overflow', as we do computation with the size
// in the fastpath. I don't think we want to do any extra check in the fastpath. There is
// nothing we can do with it without sacrificing performance.
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
#[ignore]
pub fn allocate_max_size_object_after_succeed() {
    with_mockvm(
        default_setup,
        || {
            MUTATOR.with_fixture_mut(|fixture| {
                // Allocate something so we have a thread local allocation buffer
                alloc_default_or_large(
                    fixture.mmtk(),
                    &mut fixture.mutator,
                    8,
                    8,
                    0,
                    AllocationSemantics::Default,
                );
                // Allocate an unrealistically large object
                alloc_default_or_large(
                    fixture.mmtk(),
                    &mut fixture.mutator,
                    usize::MAX,
                    8,
                    0,
                    AllocationSemantics::Default,
                );
            })
        },
        no_cleanup,
    )
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_unrealistically_large_object() {
    with_mockvm(
        default_setup,
        || {
            const CHUNK: usize = 4 * 1024 * 1024; // 4MB
                                                  // Leave some room, so we won't have arithmetic overflow when we compute size and do alignment.
            let (size, align) = (
                crate::util::conversions::raw_align_down(usize::MAX - CHUNK, 4096),
                8,
            );

            MUTATOR.with_fixture_mut(|fixture| {
                alloc_default_or_large(
                    fixture.mmtk(),
                    &mut fixture.mutator,
                    size,
                    align,
                    0,
                    AllocationSemantics::Default,
                );
            })
        },
        no_cleanup,
    )
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_more_than_heap_size() {
    with_mockvm(
        default_setup,
        || {
            // The heap has 1 MB. Allocating with 2MB will cause OOM.
            MUTATOR.with_fixture_mut(|fixture| {
                alloc_default_or_large(
                    fixture.mmtk(),
                    &mut fixture.mutator,
                    2 * 1024 * 1024,
                    8,
                    0,
                    AllocationSemantics::Default,
                );
            })
        },
        no_cleanup,
    )
}
