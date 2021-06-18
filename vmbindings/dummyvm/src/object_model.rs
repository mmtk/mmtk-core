use mmtk::util::metadata::{HeaderMetadataSpec, MetadataSpec};
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::ObjectModel;
use mmtk::AllocationSemantics;
use mmtk::CopyContext;
use std::sync::atomic::Ordering;
use DummyVM;

pub struct VMObjectModel {}

const DUMMY_METADATA: MetadataSpec = MetadataSpec::InHeader(HeaderMetadataSpec {
    bit_offset: 0,
    num_of_bits: 0,
});

impl ObjectModel<DummyVM> for VMObjectModel {
    const GLOBAL_LOG_BIT_SPEC: MetadataSpec = DUMMY_METADATA;
    const LOCAL_FORWARDING_POINTER_SPEC: MetadataSpec = DUMMY_METADATA;
    const LOCAL_FORWARDING_BITS_SPEC: MetadataSpec = DUMMY_METADATA;
    const LOCAL_MARK_BIT_SPEC: MetadataSpec = DUMMY_METADATA;
    const LOCAL_LOS_MARK_NURSERY_SPEC: MetadataSpec = DUMMY_METADATA;
    const LOCAL_UNLOGGED_BIT_SPEC: MetadataSpec = DUMMY_METADATA;

    fn load_metadata(
        _metadata_spec: MetadataSpec,
        _object: ObjectReference,
        _mask: Option<usize>,
        _atomic_ordering: Option<Ordering>,
    ) -> usize {
        unimplemented!()
    }

    fn store_metadata(
        _metadata_spec: MetadataSpec,
        _object: ObjectReference,
        _val: usize,
        _mask: Option<usize>,
        _atomic_ordering: Option<Ordering>,
    ) {
        unimplemented!()
    }

    fn compare_exchange_metadata(
        _metadata_spec: MetadataSpec,
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
        _metadata_spec: MetadataSpec,
        _object: ObjectReference,
        _val: usize,
        _order: Ordering,
    ) -> usize {
        unimplemented!()
    }

    fn fetch_sub_metadata(
        _metadata_spec: MetadataSpec,
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
}
