use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;

/// VM-specific methods for reference processing, including weak references, and finalizers.
/// We handle weak references and finalizers differently:
/// * for weak references, we assume they are implemented as normal reference objects (also known as weak objects)
///   with a referent that is actually weakly reachable. This trait provides a few methods to access
///   the referent of such an reference object.
/// * for finalizers, we provide a `Finalizable` trait, and require bindings to specify a type
///   that implements `Finalizable`. When the binding registers or pops a finalizable object
///   from MMTk, the specified type is used for the finalizable objects. For most languages,
///   they can just use `ObjectReference` for the finalizable type, meaning that they are registering
///   and popping a normal object reference as finalizable objects.
pub trait ReferenceGlue<VM: VMBinding> {
    /// The type of finalizable objects. This type is used when the binding registers and pops finalizable objects.
    type FinalizableType: Finalizable;

    // TODO: Should we also move the following methods about weak references to a trait (similar to the `Finalizable` trait)?

    /// Weak and soft references always clear the referent
    /// before enqueueing.
    ///
    /// Arguments:
    /// * `new_reference`: The reference whose referent is to be cleared.
    fn clear_referent(new_reference: ObjectReference) {
        Self::set_referent(new_reference, ObjectReference::NULL);
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

    /// Check if the referent has been cleared.
    ///
    /// Arguments:
    /// * `referent`: The referent object reference.
    fn is_referent_cleared(referent: ObjectReference) -> bool {
        referent.is_null()
    }

    /// For weak reference types, if the referent is cleared during GC, the reference
    /// will be added to a queue, and MMTk will call this method to inform
    /// the VM about the changes for those references. This method is used
    /// to implement Java's ReferenceQueue.
    /// Note that this method is called for each type of weak references during GC, and
    /// the references slice will be cleared after this call is returned. That means
    /// MMTk will no longer keep these references alive once this method is returned.
    fn enqueue_references(references: &[ObjectReference], tls: VMWorkerThread);
}

use crate::scheduler::gc_work::ProcessEdgesWork;

/// A finalizable object for MMTk. MMTk needs to know the actual object reference in the type,
/// while a binding can use this type to store some runtime information about finalizable objects.
/// For example, for bindings that allows multiple finalizer methods with one object, they can define
/// the type as a tuple of `(object, finalize method)`, and register different finalizer methods to MMTk
/// for the same object.
/// The implementation should mark theird method implementations as inline for performance.
pub trait Finalizable: std::fmt::Debug + Send {
    /// Load the object reference.
    fn get_reference(&self) -> ObjectReference;
    /// Store the object reference.
    fn set_reference(&mut self, object: ObjectReference);
    /// Keep the heap references in the finalizable object alive. For example, the reference itself needs to be traced. However,
    /// if the finalizable object includes other heap references, the implementation should trace them as well.
    /// Note that trace_object() may move objects so we need to write the new reference in case that it is moved.
    fn keep_alive<E: ProcessEdgesWork>(&mut self, trace: &mut E);
}

/// This provides an implementation of `Finalizable` for `ObjectReference`. Most bindings
/// should be able to use `ObjectReference` as `ReferenceGlue::FinalizableType`.
impl Finalizable for ObjectReference {
    #[inline(always)]
    fn get_reference(&self) -> ObjectReference {
        *self
    }
    #[inline(always)]
    fn set_reference(&mut self, object: ObjectReference) {
        *self = object;
    }
    #[inline(always)]
    fn keep_alive<E: ProcessEdgesWork>(&mut self, trace: &mut E) {
        *self = trace.trace_object(*self);
    }
}
