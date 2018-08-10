use ::util::ObjectReference;

/**
 * VM-specific stuff for util::ReferenceProcessor
 * a.k.a Pavel gets fed up with OOP
 */
pub trait ReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference);
}