// GITHUB-CI: MMTK_PLAN=all

use crate::memory_manager;
use crate::util::alloc::AllocatorInfo;
use crate::util::options::PlanSelector;
use crate::util::test_util::fixtures::*;
use crate::util::test_util::mock_vm::*;
use crate::AllocationSemantics;

#[test]
pub fn test_allocator_info() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MMTKFixture::create();

            let selector = memory_manager::get_allocator_mapping(
                fixture.get_mmtk(),
                AllocationSemantics::Default,
            );
            let base_offset = crate::plan::Mutator::<MockVM>::get_allocator_base_offset(selector);
            let allocator_info = AllocatorInfo::new::<MockVM>(selector);

            match *fixture.get_mmtk().get_options().plan {
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
                        base_offset + crate::util::constants::BYTES_IN_ADDRESS,
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
        },
        no_cleanup,
    )
}
