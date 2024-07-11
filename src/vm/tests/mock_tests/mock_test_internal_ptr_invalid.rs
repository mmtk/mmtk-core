// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::*;

#[test]
pub fn interior_pointer_invalid() {
    const MB: usize = 1024 * 1024;
    with_mockvm(
        default_setup,
        || {
            // Set up MMTk even if we don't use it.
            let _ = MutatorFixture::create_with_heapsize(10 * MB);

            let assert_no_object = |addr: Address| {
                let base_ref = crate::memory_manager::find_object_from_internal_pointer::<MockVM>(
                    addr,
                    usize::MAX,
                );
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
        no_cleanup,
    )
}
