use crate::plan::TraceLocal;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::scheduler::ProcessEdgesWork;

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

    fn enqueue_reference(object: ObjectReference);
}
