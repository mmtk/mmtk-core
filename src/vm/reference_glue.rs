use crate::plan::TraceLocal;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;

/// VM-specific methods for reference processing.
pub trait ReferenceGlue<VM: VMBinding> {
    /// Weak and soft references always clear the referent
    /// before enqueueing.
    ///
    /// Arguments:
    /// * `new_reference`: The reference whose referent is to be cleared.
    fn clear_referent(new_reference: ObjectReference) {
        Self::set_referent(new_reference, unsafe {
            Address::zero().to_object_reference()
        });
    }

    /// Get the referent from a reference.
    ///
    /// Arguments:
    /// * `object`: The object reference.
    fn get_referent(object: ObjectReference) -> ObjectReference;

    /// Set the referent in a reference.
    ///
    /// Arguments:
    /// * `reff`: The object reference for the reference.
    /// * `referent`: The referent object reference.
    fn set_referent(reff: ObjectReference, referent: ObjectReference);

    /// Process a reference with the current semantics and return an updated reference (e.g. with a new address)
    /// if the reference is still alive, otherwise return a null object reference.
    ///
    /// Arguments:
    /// * `trace`: A reference to a `TraceLocal` object for this reference.
    /// * `reference`: The address of the reference. This may or may not be the address of a heap object, depending on the VM.
    /// * `tls`: The GC thread that is processing this reference.
    fn process_reference<T: TraceLocal>(
        trace: &mut T,
        reference: ObjectReference,
        tls: VMWorkerThread,
    ) -> ObjectReference;
}
