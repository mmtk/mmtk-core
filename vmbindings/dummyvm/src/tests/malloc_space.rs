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

#[test]
pub fn malloc_free() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, 40, 8, 0, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        assert!(res.is_aligned_to(8));
        mmtk_free(res);
    })
}

#[test]
pub fn malloc_offset() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, 40, 8, 4, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        assert!((res + 4usize).is_aligned_to(8));
        mmtk_free(res);
    })
}

#[test]
pub fn malloc_usable_size() {
    MUTATOR.with_fixture(|fixture| {
        let res = mmtk_alloc(fixture.mutator, 40, 8, 0, AllocationSemantics::Malloc);
        assert!(!res.is_zero());
        let size = mmtk_malloc_usable_size(res);
        assert_eq!(size, 40);
        mmtk_free(res);
    })
}
