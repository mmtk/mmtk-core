use mmtk::vm::ObjectModel;
use mmtk::util::{Address, ObjectReference};
use mmtk::AllocationSemantics;
use mmtk::CopyContext;
use DummyVM;

pub struct VMObjectModel {}

impl ObjectModel<DummyVM> for VMObjectModel {
    const GC_BYTE_OFFSET: isize = 0;
    
    fn copy(_from: ObjectReference, _semantics: AllocationSemantics, _copy_context: &mut impl CopyContext) -> ObjectReference {
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
