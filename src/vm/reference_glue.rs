use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;

/// VM-specific methods for reference processing.
pub trait ReferenceGlue<VM: VMBinding> {
    type FinalizableType: Finalizable;

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

    /// For reference types, if the referent is cleared during GC, the reference
    /// will be added to a queue, and MMTk will call this method to inform
    /// the VM about the changes for those references. This method is used
    /// to implement Java's ReferenceQueue.
    /// Note that this method is called for each type of weak references during GC, and
    /// the references slice will be cleared after this call is returned. That means
    /// MMTk will no longer keep these references alive once this method is returned.
    fn enqueue_references(references: &[ObjectReference], tls: VMWorkerThread);
}

pub trait Finalizable: std::fmt::Debug + Send {
    fn load_reference(&self) -> ObjectReference;
    fn set_reference(&mut self, object: ObjectReference);
}

impl Finalizable for ObjectReference {
    #[inline(always)]
    fn load_reference(&self) -> ObjectReference {
        *self
    }
    #[inline(always)]
    fn set_reference(&mut self, object: ObjectReference) {
        *self = object;
    }
}
