use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;

/// VM-specific methods for reference processing, including weak references, and finalizers.
/// We handle weak references and finalizers differently:
/// * for weak references, we assume they are implemented as normal reference objects with a
///   referent that is actually weakly reachable. This trait provides a few methods to access
///   the referent of such an reference object.
/// * for finalizers, we provide a `Finalizable` trait, and require bindings to specify a type
///   that implements `Finalizable`. When the binding registers or pops a finalizable object
///   from MMTk, the specified type is used for the finalizable objects. For most languages,
///   they can just use `ObjectReference` for the finalizable type, meaning that they are registering
///   and popping a normal object reference as finalizable objects.
pub trait ReferenceGlue<VM: VMBinding> {
    /// The type of finalizable objects. This type is used when the binding registers and pops finalizable objects.
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

    /// Get the referent from a weak reference object.
    ///
    /// Arguments:
    /// * `object`: The object reference.
    fn get_referent(object: ObjectReference) -> ObjectReference;

    /// Set the referent in a weak reference object.
    ///
    /// Arguments:
    /// * `reff`: The object reference for the reference.
    /// * `referent`: The referent object reference.
    fn set_referent(reff: ObjectReference, referent: ObjectReference);

    /// For weak reference types, if the referent is cleared during GC, the reference
    /// will be added to a queue, and MMTk will call this method to inform
    /// the VM about the changes for those references. This method is used
    /// to implement Java's ReferenceQueue.
    /// Note that this method is called for each type of weak references during GC, and
    /// the references slice will be cleared after this call is returned. That means
    /// MMTk will no longer keep these references alive once this method is returned.
    fn enqueue_references(references: &[ObjectReference], tls: VMWorkerThread);
}

/// A finalizable object for MMTk. MMTk needs to know the actual object reference in the type,
/// while a binding can use this type to store some runtime information about finalizable objects.
/// For example, for bindings that allows multiple finalizer methods with one object, they can define
/// the type as a tuple of `(object, finalize method)`, and register different finalizer methods to MMTk
/// for the same object.
pub trait Finalizable: std::fmt::Debug + Send {
    /// Load the object reference.
    fn load_reference(&self) -> ObjectReference;
    /// Store the object reference.
    fn set_reference(&mut self, object: ObjectReference);
}

/// This provides an implementation of `Finalizable` for `ObjectReference`. Most bindings
/// should be able to use `ObjectReference` as `ReferenceGlue::FinalizableType`.
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
