use crate::plan::Mutator;
use crate::scheduler::GCWorker;
use crate::util::opaque_pointer::*;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::ObjectQueue;

/// VM-specific methods for the current plan.
pub trait ActivePlan<VM: VMBinding> {
    /// Return whether there is a mutator created and associated with the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is valid (a value passed in by the VM binding through API).
    fn is_mutator(tls: VMThread) -> bool;

    /// Return a `Mutator` reference for the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is a mutator thread.
    fn mutator(tls: VMMutatorThread) -> &'static mut Mutator<VM>;

    /// Return an iterator that includes all the mutators at the point of invocation.
    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<VM>> + 'a>;

    /// Return the total count of mutators.
    fn number_of_mutators() -> usize;

    /// The fallback for object tracing. MMTk generally expects to find an object in one of MMTk's spaces (if it is allocated by MMTK),
    /// and apply the corresponding policy to trace the object. Tracing in MMTk means identifying whether we have encountered this object in the
    /// current GC. For example, for mark sweep, we will check if an object is marked, and if it is not yet marked, mark and enqueue the object
    /// for later scanning. For copying policies, copying also happens in this step. For example for MMTk's copying space, we will
    /// copy an object if it is in 'from space', and enqueue the copied object for later scanning.
    ///
    /// If a binding would like to trace objects that are not allocated by MMTk and are not in any MMTk space, they can override this method.
    /// They should check whether the object is encountered before in this current GC. If not, they should record the object as encountered themselves,
    /// and enqueue the object reference to the object queue provided by the argument. If a binding moves objects, they should do the copying in the method,
    /// and enqueue the new object reference instead.
    ///
    /// The method should return the new object reference if the method moves the object, otherwise return the original object reference.
    ///
    /// Arguments:
    /// * `queue`: The object queue. If an object is encountered for the first time in this GC, we expect the implementation to call `queue.enqueue()`
    ///            for the object. If the object is moved during the tracing, the new object reference (after copying) should be enqueued instead.
    /// * `object`: The object to trace.
    /// * `worker`: The GC worker that is doing this tracing. This is used to copy object (see [`crate::vm::ObjectModel::copy`])
    fn vm_trace_object<Q: ObjectQueue>(
        _queue: &mut Q,
        object: ObjectReference,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        panic!("MMTk cannot trace object {:?} as it does not belong to any MMTk space. If the object is known to the VM, the binding can override this method and handle its tracing.", object)
    }
}
