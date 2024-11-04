// GITHUB-CI: MMTK_PLAN=NoGC
// GITHUB-CI: FEATURES=nogc_lock_free

use super::mock_test_prelude::*;

use crate::plan::AllocationSemantics;
use crate::vm::VMBinding;

#[test]
pub fn nogc_lock_free_allocate() {
    with_mockvm(
        default_setup,
        || {
            let mut fixture = MutatorFixture::create();
            let min = MockVM::MIN_ALIGNMENT;
            let max = MockVM::MAX_ALIGNMENT;
            log::info!("Allowed alignment between {} and {}", min, max);
            let mut align = min;
            while align <= max {
                log::info!("Test allocation with alignment {}", align);
                let addr = memory_manager::alloc(
                    &mut fixture.mutator,
                    8,
                    align,
                    0,
                    AllocationSemantics::Default,
                );
                log::info!("addr = {}", addr);
                assert!(
                    addr.is_aligned_to(align),
                    "Expected allocation alignment {}, returned address is {:?}",
                    align,
                    addr
                );
                align *= 2;
            }
        },
        no_cleanup,
    )
}
