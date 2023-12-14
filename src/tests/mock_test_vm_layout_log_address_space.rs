// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use super::mock_test_vm_layout_default::test_with_vm_layout;
use crate::util::heap::vm_layout::VMLayout;

#[test]
fn test_vm_layout_log_address_space() {
    with_mockvm(
        default_setup,
        || {
            let layout = VMLayout {
                #[cfg(target_pointer_width = "32")]
                log_address_space: 31,
                #[cfg(target_pointer_width = "64")]
                log_address_space: 45,
                // Use default for the rest.
                ..VMLayout::default()
            };
            test_with_vm_layout(Some(layout));
        },
        no_cleanup,
    )
}
