// GITHUB-CI: MMTK_PLAN=all

use crate::api;
use crate::DummyVM;
use crate::tests::fixtures::{Fixture, MutatorFixture};
use mmtk::plan::AllocationSemantics;
use mmtk::vm::VMBinding;
use log::info;

lazy_static! {
    static ref MUTATOR: Fixture<MutatorFixture> = Fixture::new();
}

#[test]
pub fn allocate_alignment() {
    MUTATOR.with_fixture(|fixture| {
        let min = DummyVM::MIN_ALIGNMENT;
        let max = DummyVM::MAX_ALIGNMENT;
        info!("Allowed alignment between {} and {}", min, max);
        let mut align = min;
        while align <= max {
            info!("Test allocation with alignment {}", align);
            let addr = api::mmtk_alloc(fixture.mutator, 8, align, 0, AllocationSemantics::Default);
            assert!(addr.is_aligned_to(align), "Expected allocation alignment {}, returned address is {:?}", align, addr);
            align *= 2;
        }
    })
}

#[test]
pub fn allocate_offset() {
    MUTATOR.with_fixture(|fixture| {
        const OFFSET: isize = 4;
        let min = DummyVM::MIN_ALIGNMENT;
        let max = DummyVM::MAX_ALIGNMENT;
        info!("Allowed alignment between {} and {}", min, max);
        let mut align = min;
        while align <= max {
            info!("Test allocation with alignment {} and offset {}", align, OFFSET);
            let addr = api::mmtk_alloc(fixture.mutator, 8, align, OFFSET, AllocationSemantics::Default);
            assert!((addr + OFFSET).is_aligned_to(align), "Expected allocation alignment {}, returned address is {:?}", align, addr);
            align *= 2;
        }
    })
}
