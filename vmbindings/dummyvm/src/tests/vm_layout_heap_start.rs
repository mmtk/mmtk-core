// GITHUB-CI: MMTK_PLAN=all

use crate::tests::vm_layout_default::test_with_vm_layout;
use mmtk::util::heap::vm_layout::VMLayout;
use mmtk::util::Address;

#[test]
fn test_vm_layout_heap_start() {
    let default = VMLayout::default();

    // Test with an start address that is different to the default heap start
    #[cfg(target_pointer_width = "32")]
    let heap_start = unsafe { Address::from_usize(0x7000_0000) };
    #[cfg(target_pointer_width = "64")]
    let heap_start = unsafe { Address::from_usize(0x0000_0400_0000_0000usize) };
    #[cfg(target_pointer_width = "64")]
    assert!(heap_start.is_aligned_to(default.max_space_extent()));

    let layout = VMLayout {
        heap_start,
        // Use default for the rest.
        ..default
    };
    test_with_vm_layout(Some(layout));
}
