// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;
use crate::util::heap::vm_layout::VMLayout;

pub fn test_with_vm_layout(layout: Option<VMLayout>) {
    use crate::plan::AllocationSemantics;

    let mut fixture = MutatorFixture::create_with_builder(|builder| {
        // 1MB
        builder
            .options
            .gc_trigger
            .set(crate::util::options::GCTriggerSelector::FixedHeapSize(
                1024 * 1024,
            ));
        // Set layout
        if let Some(layout) = layout {
            builder.set_vm_layout(layout);
        }
    });

    // Test allocation
    let addr = memory_manager::alloc(&mut fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
    let obj = <MockVM as VMBinding>::VMObjectModel::address_to_ref(addr);
    // Test SFT
    assert!(memory_manager::is_in_mmtk_spaces::<MockVM>(obj));
    // Test mmapper
    assert!(memory_manager::is_mapped_address(addr));
}

#[test]
fn test_vm_layout_default() {
    with_mockvm(
        default_setup,
        || {
            test_with_vm_layout(None);
        },
        no_cleanup,
    )
}
