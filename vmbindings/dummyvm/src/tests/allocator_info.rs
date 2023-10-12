// GITHUB-CI: MMTK_PLAN=all

use mmtk::util::alloc::AllocatorInfo;
use mmtk::util::options::PlanSelector;
use mmtk::AllocationSemantics;

use crate::test_fixtures::{Fixture, MMTKSingleton};
use crate::DummyVM;

lazy_static! {
    static ref MMTK_SINGLETON: Fixture<MMTKSingleton> = Fixture::new();
}

#[test]
fn test_allocator_info() {
    MMTK_SINGLETON.with_fixture(|fixture| {
        let selector = mmtk::memory_manager::get_allocator_mapping(
            &fixture.mmtk,
            AllocationSemantics::Default,
        );
        let base_offset = mmtk::plan::Mutator::<DummyVM>::get_allocator_base_offset(selector);
        let allocator_info = AllocatorInfo::new::<DummyVM>(selector);

        match *fixture.mmtk.get_options().plan {
            PlanSelector::NoGC
            | PlanSelector::Immix
            | PlanSelector::SemiSpace
            | PlanSelector::GenCopy
            | PlanSelector::GenImmix
            | PlanSelector::MarkCompact
            | PlanSelector::StickyImmix => {
                // These plans all use bump pointer allocator.
                let AllocatorInfo::BumpPointer {
                    bump_pointer_offset,
                } = allocator_info
                else {
                    panic!("Expected AllocatorInfo for a bump pointer allocator");
                };
                // In all of those plans, the first field at base offset is tls, and the second field is the BumpPointer struct.
                assert_eq!(
                    base_offset + mmtk::util::constants::BYTES_IN_ADDRESS,
                    bump_pointer_offset
                );
            }
            PlanSelector::MarkSweep => {
                if cfg!(feature = "malloc_mark_sweep") {
                    // We provide no info for a malloc allocator
                    assert!(matches!(allocator_info, AllocatorInfo::None))
                } else {
                    // We haven't implemented for a free list allocator
                    assert!(matches!(allocator_info, AllocatorInfo::Unimplemented))
                }
            }
            // We provide no info for a large object allocator
            PlanSelector::PageProtect => assert!(matches!(allocator_info, AllocatorInfo::None)),
        }
    })
}
