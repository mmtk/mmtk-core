// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=malloc_space

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
