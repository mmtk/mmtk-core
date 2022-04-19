// This runs with plan that is not malloc MS
// GITHUB-CI: MMTK_PLAN=NoGC
// GITHUB-CI: MMTK_PLAN=Immix
// GITHUB-CI: MMTK_PLAN=GenImmix
// GITHUB-CI: MMTK_PLAN=GenCopy
// GITHUB-CI: MMTK_PLAN=MarkCompact
// GITHUB-CI: FEATURES=malloc_space,nogc_multi_space

use crate::api::*;
use crate::tests::fixtures::{Fixture, MutatorInstance};

use mmtk::AllocationSemantics;

lazy_static! {
    static ref MUTATOR: Fixture<MutatorInstance> = Fixture::new();
}

const SIZE: usize = 40;
const NO_OFFSET: isize = 0;
const ALIGN: usize = 8;
const OFFSET: isize = 4;

#[test]
pub fn malloc_free() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, SIZE, ALIGN, NO_OFFSET, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        assert!(res.is_aligned_to(ALIGN));
        mmtk_free(res);
    })
}

#[test]
pub fn malloc_offset() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, SIZE, ALIGN, OFFSET, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        assert!((res + OFFSET).is_aligned_to(ALIGN));
        mmtk_free(res);
    })
}

#[test]
pub fn malloc_usable_size() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, SIZE, ALIGN, NO_OFFSET, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        let size = mmtk_malloc_usable_size(res);
        assert!(size >= SIZE);
        mmtk_free(res);
    })
}
