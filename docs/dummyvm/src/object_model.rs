use crate::DummyVM;
use mmtk::util::copy::{CopySemantics, GCWorkerCopyContext};
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::*;

pub struct VMObjectModel {}

/// This is the offset from the allocation result to the object reference for the object.
/// For bindings that this offset is not a constant, you can implement the calculation in the method `ref_to_object_start``, and
/// remove this constant.
pub const OBJECT_REF_OFFSET: usize = 0;

/// This is the offset from the object reference to an in-object address. The binding needs
/// to guarantee the in-object address is inside the storage associated with the object.
/// It has to be a constant offset. See `ObjectModel::IN_OBJECT_ADDRESS_OFFSET`.
pub const IN_OBJECT_ADDRESS_OFFSET: isize = 0;

// This is the offset from the object reference to the object header.
// This value is used in `ref_to_header` where MMTk loads header metadata from.
pub const OBJECT_HEADER_OFFSET: usize = 0;

// Documentation: https://docs.mmtk.io/api/mmtk/vm/object_model/trait.ObjectModel.html
impl ObjectModel<DummyVM> for VMObjectModel {
    // Global metadata

    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::side_first();

    // Local metadata

    // Forwarding pointers have to be in the header. It is okay to overwrite the object payload with a forwarding pointer.
    // FIXME: The bit offset needs to be set properly.
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec =
        VMLocalForwardingPointerSpec::in_header(0);
    // The other metadata can be put in the side metadata.
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
        VMLocalForwardingBitsSpec::side_first();
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec =
        VMLocalMarkBitSpec::side_after(Self::LOCAL_FORWARDING_BITS_SPEC.as_spec());
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec =
        VMLocalLOSMarkNurserySpec::side_after(Self::LOCAL_MARK_BIT_SPEC.as_spec());

    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = OBJECT_REF_OFFSET as isize;

    fn copy(
        _from: ObjectReference,
        _semantics: CopySemantics,
        _copy_context: &mut GCWorkerCopyContext<DummyVM>,
    ) -> ObjectReference {
        unimplemented!()
    }

    fn copy_to(_from: ObjectReference, _to: ObjectReference, _region: Address) -> Address {
        unimplemented!()
    }

    fn get_current_size(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_size_when_copied(object: ObjectReference) -> usize {
        // FIXME: This assumes the object size is unchanged during copying.
        Self::get_current_size(object)
    }

    fn get_align_when_copied(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_align_offset_when_copied(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_reference_when_copied_to(_from: ObjectReference, _to: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_type_descriptor(_reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn ref_to_object_start(object: ObjectReference) -> Address {
        object.to_raw_address().sub(OBJECT_REF_OFFSET)
    }

    fn ref_to_header(object: ObjectReference) -> Address {
        object.to_raw_address().sub(OBJECT_HEADER_OFFSET)
    }

    const IN_OBJECT_ADDRESS_OFFSET: isize = IN_OBJECT_ADDRESS_OFFSET;

    fn dump_object(_object: ObjectReference) {
        unimplemented!()
    }
}
