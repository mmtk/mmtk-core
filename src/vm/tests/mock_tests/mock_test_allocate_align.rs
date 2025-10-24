// GITHUB-CI: MMTK_PLAN=all

use lazy_static::lazy_static;

use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;

lazy_static! {
    static ref MUTATOR: Fixture<MutatorFixture> = Fixture::new();
}

#[test]
pub fn allocate_alignment() {
    with_mockvm(
        default_setup,
        || {
            MUTATOR.with_fixture_mut(|fixture| {
                let min = MockVM::MIN_ALIGNMENT;
                let max = MockVM::MAX_ALIGNMENT;
                info!("Allowed alignment between {} and {}", min, max);
                let mut align = min;
                while align <= max {
                    info!("Test allocation with alignment {}", align);
                    let addr = memory_manager::alloc(
                        fixture.mutator(),
                        8,
                        align,
                        0,
                        AllocationSemantics::Default,
                    );
                    assert!(
                        addr.is_aligned_to(align),
                        "Expected allocation alignment {}, returned address is {:?}",
                        align,
                        addr
                    );
                    align *= 2;
                }
            })
        },
        no_cleanup,
    )
}
