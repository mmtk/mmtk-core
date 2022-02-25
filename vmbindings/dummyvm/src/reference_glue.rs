use mmtk::vm::ReferenceGlue;
use mmtk::util::ObjectReference;
use mmtk::TraceLocal;
use mmtk::util::opaque_pointer::*;
use crate::DummyVM;

pub struct VMReferenceGlue {}

impl ReferenceGlue<DummyVM> for VMReferenceGlue {
    fn set_referent(_reference: ObjectReference, _referent: ObjectReference) {
        unimplemented!()
    }
    fn get_referent(_object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }
    fn process_reference<T: TraceLocal>(_trace: &mut T, _reference: ObjectReference, _tls: VMWorkerThread) -> ObjectReference {
        unimplemented!()
    }
}
