// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::constants::LOG_BITS_IN_WORD;
use crate::util::is_mmtk_object::VO_BIT_REGION_SIZE;
use crate::util::*;
use crate::AllocationSemantics;
use crate::vm::ObjectModel;

#[test]
pub fn interior_poiner_invalid() {
    const MB: usize = 1024 * 1024;
    with_mockvm(
        default_setup,
        || {
            let mut fixture = MutatorFixture::create_with_heapsize(10 * MB);

            let mut assert_no_object = |addr: Address| {
                let base_ref = crate::memory_manager::find_object_from_internal_pointer::<MockVM>(addr, usize::MAX);
                assert!(base_ref.is_none());
            };

            let heap_start = crate::util::heap::layout::vm_layout::vm_layout().heap_start;
            for offset in 0..16usize {
                let addr = heap_start + offset;
                assert_no_object(addr);
            }

            let heap_end = crate::util::heap::layout::vm_layout::vm_layout().heap_end;
            for offset in 0..16usize {
                let addr = heap_end - offset;
                assert_no_object(addr);
            }
        },
        no_cleanup
    )
}
