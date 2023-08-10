// GITHUB-CI: MMTK_PLAN=all

use crate::api;
use crate::tests::fixtures::{SerialFixture, MutatorFixture};
use mmtk::plan::AllocationSemantics;

lazy_static! {
    static ref MUTATOR: SerialFixture<MutatorFixture> = SerialFixture::new();
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_unrealistically_large_object() {
    #[cfg(target_pointer_width = "64")]
    let (size, align) = (2251799813685249 * 4096, 8);
    #[cfg(target_pointer_width = "32")]
    let (size, align) = (mmtk::util::conversions::raw_align_down(usize::MAX, 4096), 4);

    MUTATOR.with_fixture(|fixture| {
        api::mmtk_alloc(fixture.mutator, size, align, 0, AllocationSemantics::Default);
    })
}

#[test]
#[should_panic(expected = "Out of memory with HeapOutOfMemory!")]
pub fn allocate_more_than_heap_size() {
    // The heap has 1 MB. Allocating with 2MB will cause OOM.
    MUTATOR.with_fixture(|fixture| {
        api::mmtk_alloc(fixture.mutator, 2 * 1024 * 1024, 8, 0, AllocationSemantics::Default);
    })
}
