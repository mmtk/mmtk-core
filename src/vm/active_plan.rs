use crate::plan::Plan;
use crate::scheduler::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::MutexGuard;
use crate::plan::global::PlanTypes;
use crate::plan::Mutator;

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
    fn global() -> &'static dyn Plan<VM=VM>;

    /// Return a `GCWorker` reference for the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is a GC worker thread.
    unsafe fn worker(tls: OpaquePointer) -> &'static mut GCWorker<VM>;

    /// Return whether there is a mutator created and associated with the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is valid (a value passed in by the VM binding through API).
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;

    /// Return a `Mutator` reference for the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is a mutator thread.
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut Mutator<VM>;

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
}
