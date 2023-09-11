// GITHUB-CI: MMTK_PLAN=all

use mmtk::util::heap::vm_layout::VMLayout;

pub fn test_with_vm_layout(layout: Option<VMLayout>) {
    use crate::api;
    use crate::test_fixtures::VMLayoutFixture;
    use mmtk::plan::AllocationSemantics;
    use mmtk::vm::ObjectModel;

    let fixture = VMLayoutFixture::create_with_layout(layout);

    // Test allocation
    let addr = api::mmtk_alloc(fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
    let obj = crate::object_model::VMObjectModel::address_to_ref(addr);
    // Test SFT
    assert!(api::mmtk_is_in_mmtk_spaces(obj));
    // Test mmapper
    assert!(api::mmtk_is_mapped_address(addr));
}

#[test]
fn test_vm_layout_default() {
    test_with_vm_layout(None::<VMLayout>);
}
