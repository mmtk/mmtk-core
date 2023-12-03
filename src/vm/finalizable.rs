use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::util::ObjectReference;

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
    fn get_reference(&self) -> ObjectReference {
        *self
    }
    fn set_reference(&mut self, object: ObjectReference) {
        *self = object;
    }
    fn keep_alive<E: ProcessEdgesWork>(&mut self, trace: &mut E) {
        *self = trace.trace_object(*self);
    }
}
