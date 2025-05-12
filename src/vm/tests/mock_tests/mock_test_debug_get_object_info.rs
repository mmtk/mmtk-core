// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

#[test]
pub fn debug_print_object_info() {
    with_mockvm(
        default_setup,
        || {
            let fixture = SingleObject::create();
            crate::mmtk::mmtk_debug_print_object_info(fixture.objref);
        },
        no_cleanup,
    )
}
