use ::vm::ReferenceGlue;
use ::util::ObjectReference;
use ::plan::TraceLocal;
use ::util::OpaquePointer;
use libc::c_void;

pub struct VMReferenceGlue {}

impl ReferenceGlue for VMReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        unimplemented!()
    }
    fn get_referent(object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }
    fn process_reference<T: TraceLocal>(trace: &mut T, reference: ObjectReference, tls: OpaquePointer) -> ObjectReference {
        unimplemented!()
    }
}