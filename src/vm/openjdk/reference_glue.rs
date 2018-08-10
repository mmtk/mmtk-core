use ::vm::ReferenceGlue;
use ::util::ObjectReference;

pub struct VMReferenceGlue {}

impl ReferenceGlue for VMReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        unimplemented!()
    }
}