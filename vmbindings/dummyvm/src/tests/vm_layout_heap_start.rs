// GITHUB-CI: MMTK_PLAN=all

use crate::tests::vm_layout_default::test_with_vm_layout;
use mmtk::util::heap::vm_layout::VMLayout;
use mmtk::util::Address;

#[test]
fn test_vm_layout_heap_start() {
    let default = VMLayout::default();

    // Test with an address that is smaller than the default heap size
    #[cfg(target_pointer_width = "32")]
    let before_default_heap_start = unsafe { Address::from_usize(0x7000_0000) };
    #[cfg(target_pointer_width = "64")]
    let before_default_heap_start = unsafe { Address::from_usize(0x0000_0100_0000_0000usize) };
    assert!(before_default_heap_start < default.heap_start);

    let layout = VMLayout {
        heap_start: before_default_heap_start,
        // Use default for the rest.
        ..default
    };
    test_with_vm_layout(Some(layout));
}
