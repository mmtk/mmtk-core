use crate::plan::Mutator;
use crate::plan::Plan;
use crate::scheduler::GCWorker;
use crate::util::opaque_pointer::*;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::ObjectQueue;
use std::marker::PhantomData;
use std::sync::MutexGuard;

pub struct SynchronizedMutatorIterator<'a, VM: VMBinding> {
    _guard: MutexGuard<'a, ()>,
    start: bool,
    phantom: PhantomData<VM>,
}

impl<'a, VM: VMBinding> Iterator for SynchronizedMutatorIterator<'a, VM> {
    type Item = &'static mut Mutator<VM>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start {
            self.start = false;
            VM::VMActivePlan::reset_mutator_iterator();
        }
        VM::VMActivePlan::get_next_mutator()
    }
}

/// VM-specific methods for the current plan.
pub trait ActivePlan<VM: VMBinding> {
    /// Return a reference to the current plan.
    // TODO: I don't know how this can be implemented when we have multiple MMTk instances.
    // This function is used by space and phase to refer to the current plan.
    // Possibly we should remove the use of this function, and remove this function?
    fn global() -> &'static dyn Plan<VM = VM>;

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

    /// Reset the mutator iterator so that `get_next_mutator()` returns the first mutator.
    fn reset_mutator_iterator();

    /// Return the next mutator if there is any. This method assumes that the VM implements stateful type
    /// to remember which mutator is returned and guarantees to return the next when called again. This does
    /// not need to be thread safe.
    fn get_next_mutator() -> Option<&'static mut Mutator<VM>>;

    /// A utility method to provide a thread-safe mutator iterator from `reset_mutator_iterator()` and `get_next_mutator()`.
    fn mutators<'a>() -> SynchronizedMutatorIterator<'a, VM> {
        SynchronizedMutatorIterator {
            _guard: Self::global().base().mutator_iterator_lock.lock().unwrap(),
            start: true,
            phantom: PhantomData,
        }
    }

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
