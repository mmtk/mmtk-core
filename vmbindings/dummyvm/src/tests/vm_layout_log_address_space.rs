// GITHUB-CI: MMTK_PLAN=all

use crate::tests::vm_layout_default::test_with_vm_layout;
use mmtk::util::heap::vm_layout::VMLayout;

#[test]
fn test_vm_layout_log_address_space() {
    let layout = VMLayout {
        #[cfg(target_pointer_width = "32")]
        log_address_space: 31,
        #[cfg(target_pointer_width = "64")]
        log_address_space: 45,
        // Use default for the rest.
        ..VMLayout::default()
    };
    test_with_vm_layout(Some(layout));
}
