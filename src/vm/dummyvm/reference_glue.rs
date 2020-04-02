use super::DummyVM;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::ReferenceGlue;
use crate::TraceLocal;

pub struct VMReferenceGlue {}

impl ReferenceGlue<DummyVM> for VMReferenceGlue {
    fn set_referent(_reff: ObjectReference, _referent: ObjectReference) {
        unimplemented!()
    }
    fn get_referent(_object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }
    fn process_reference<T: TraceLocal>(
        _trace: &mut T,
        _reference: ObjectReference,
        _tls: OpaquePointer,
    ) -> ObjectReference {
        unimplemented!()
    }
}
