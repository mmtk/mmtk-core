use mmtk::util::metadata::header_metadata::HeaderMetadataSpec;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::*;
use mmtk::AllocationSemantics;
use mmtk::CopyContext;
use std::sync::atomic::Ordering;
use DummyVM;

pub struct VMObjectModel {}

impl ObjectModel<DummyVM> for VMObjectModel {
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::in_header(0);
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec = VMLocalForwardingPointerSpec::in_header(0);
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec = VMLocalForwardingBitsSpec::in_header(0);
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::in_header(0);
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec = VMLocalLOSMarkNurserySpec::in_header(0);

    fn load_metadata(
        _metadata_spec: &HeaderMetadataSpec,
        _object: ObjectReference,
        _mask: Option<usize>,
        _atomic_ordering: Option<Ordering>,
    ) -> usize {
        unimplemented!()
    }

    fn store_metadata(
        _metadata_spec: &HeaderMetadataSpec,
        _object: ObjectReference,
        _val: usize,
        _mask: Option<usize>,
        _atomic_ordering: Option<Ordering>,
    ) {
        unimplemented!()
    }

    fn compare_exchange_metadata(
        _metadata_spec: &HeaderMetadataSpec,
        _object: ObjectReference,
        _old_val: usize,
        _new_val: usize,
        _mask: Option<usize>,
        _success_order: Ordering,
        _failure_order: Ordering,
    ) -> bool {
        unimplemented!()
    }

    fn fetch_add_metadata(
        _metadata_spec: &HeaderMetadataSpec,
        _object: ObjectReference,
        _val: usize,
        _order: Ordering,
    ) -> usize {
        unimplemented!()
    }

    fn fetch_sub_metadata(
        _metadata_spec: &HeaderMetadataSpec,
        _object: ObjectReference,
        _val: usize,
        _order: Ordering,
    ) -> usize {
        unimplemented!()
    }

    fn copy(
        _from: ObjectReference,
        _semantics: AllocationSemantics,
        _copy_context: &mut impl CopyContext,
    ) -> ObjectReference {
        unimplemented!()
    }

    fn copy_to(_from: ObjectReference, _to: ObjectReference, _region: Address) -> Address {
        unimplemented!()
    }

    fn get_current_size(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_reference_when_copied_to(_from: ObjectReference, _to: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_type_descriptor(_reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn object_start_ref(_object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn ref_to_address(_object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn dump_object(_object: ObjectReference) {
        unimplemented!()
    }

    fn object_alignment() -> i32 { 
        8
    }
}
