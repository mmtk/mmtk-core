// GITHUB-CI: MMTK_PLAN=NoGC
// GITHUB-CI: FEATURES=nogc_lock_free

use crate::api;
use crate::test_fixtures::{MutatorFixture, SerialFixture};
use crate::DummyVM;
use log::info;
use mmtk::plan::AllocationSemantics;
use mmtk::vm::VMBinding;

lazy_static! {
    static ref MUTATOR: SerialFixture<MutatorFixture> = SerialFixture::new();
}

#[test]
pub fn nogc_lock_free_allocate() {
    MUTATOR.with_fixture(|fixture| {
        let min = DummyVM::MIN_ALIGNMENT;
        let max = DummyVM::MAX_ALIGNMENT;
        info!("Allowed alignment between {} and {}", min, max);
        let mut align = min;
        while align <= max {
            info!("Test allocation with alignment {}", align);
            let addr = api::mmtk_alloc(fixture.mutator, 8, align, 0, AllocationSemantics::Default);
            info!("addr = {}", addr);
            assert!(
                addr.is_aligned_to(align),
                "Expected allocation alignment {}, returned address is {:?}",
                align,
                addr
            );
            align *= 2;
        }
    })
}
