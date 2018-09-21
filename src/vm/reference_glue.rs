use ::util::ObjectReference;
use ::util::Address;
use ::plan::TraceLocal;

use libc::c_void;

/**
 * VM-specific stuff for util::ReferenceProcessor
 * a.k.a Pavel gets fed up with OOP
 */
pub trait ReferenceGlue {
    fn clear_referent(new_reference: ObjectReference) {
        Self::set_referent(new_reference, unsafe { Address::zero().to_object_reference() });
    }
    fn get_referent(object: ObjectReference) -> ObjectReference;
    fn set_referent(reff: ObjectReference, referent: ObjectReference);

    fn process_reference<T: TraceLocal>(trace: &mut T, reference: ObjectReference, tls: *mut c_void) -> ObjectReference;
}