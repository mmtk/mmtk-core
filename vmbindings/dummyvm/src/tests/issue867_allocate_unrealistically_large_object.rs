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
    MUTATOR.with_fixture(|fixture| {
        api::mmtk_alloc(fixture.mutator, 2251799813685249 * 4096, 8, 0, AllocationSemantics::Default);
    })
}
