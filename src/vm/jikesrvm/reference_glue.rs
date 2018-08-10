use ::vm::ReferenceGlue;
use ::util::ObjectReference;
use super::entrypoint::*;

pub struct VMReferenceGlue {}

impl ReferenceGlue for VMReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        unsafe {
            (reff.to_address() + REFERENCE_REFERENT_FIELD_OFFSET).store(referent.value());
        }
    }
}