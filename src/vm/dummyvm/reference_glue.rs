use super::DummyVM;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::ReferenceGlue;
use crate::TraceLocal;
use libc::c_void;

pub struct VMReferenceGlue {}

impl ReferenceGlue<DummyVM> for VMReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        unimplemented!()
    }
    fn get_referent(object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }
    fn process_reference<T: TraceLocal>(
        trace: &mut T,
        reference: ObjectReference,
        tls: OpaquePointer,
    ) -> ObjectReference {
        unimplemented!()
    }
}
