// LXR requires in-header forwarding bits, so it is not tested here.
// GITHUB-CI: MMTK_PLAN=NoGC,SemiSpace,GenCopy,GenImmix,MarkSweep,PageProtect,Immix,Lisp2,StickyImmix,ConcurrentImmix

use super::mock_test_prelude::*;
use crate::plan::AllocationSemantics;

define_mock_vm! {
    /// A mock VM that places all the metadata that could be on the side on the side.
    type SideMetadataVM = GenericMockVM<SideMetadataConfig> {
        const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::side_first();
        const GLOBAL_FIELD_UNLOG_BIT_SPEC: VMGlobalFieldUnlogBitSpec =
            VMGlobalFieldUnlogBitSpec::side_after(Self::GLOBAL_LOG_BIT_SPEC.as_spec());
        const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
            VMLocalForwardingBitsSpec::side_first();
        const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec =
            VMLocalMarkBitSpec::side_after(Self::LOCAL_FORWARDING_BITS_SPEC.as_spec());
        const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec =
            VMLocalLOSMarkNurserySpec::side_after(Self::LOCAL_MARK_BIT_SPEC.as_spec());
    }
}

#[test]
pub fn side_metadata_gc() {
    SideMetadataVM::with_mockvm(
        SideMetadataVM::default,
        || {
            // The constants from the config are used by the VM binding.
            assert!(
                <SideMetadataVM as ObjectModel<SideMetadataVM>>::LOCAL_FORWARDING_BITS_SPEC
                    .is_on_side()
            );
            assert!(<MockVM as ObjectModel<MockVM>>::LOCAL_FORWARDING_BITS_SPEC.is_in_header());

            // 1MB heap
            const MB: usize = 1024 * 1024;
            let fixture = GenericMutatorFixture::<SideMetadataVM>::create_with_heapsize(MB);

            // Normal alloc
            let addr =
                memory_manager::alloc(fixture.mutator(), 16, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
            info!("Allocated default at: {:#x}", addr);

            memory_manager::handle_user_collection_request(
                fixture.mmtk(),
                fixture.mutator_tls(),
                false,
            );
        },
        no_cleanup,
    )
}
