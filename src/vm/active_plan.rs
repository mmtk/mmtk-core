use crate::plan::{Plan, SelectedPlan};
use crate::scheduler::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::MutexGuard;

pub struct SynchronizedMutatorIterator<'a, VM: VMBinding> {
    _guard: MutexGuard<'a, ()>,
    start: bool,
    phantom: PhantomData<VM>,
}

impl<'a, VM: VMBinding> Iterator for SynchronizedMutatorIterator<'a, VM> {
    type Item = &'static mut <SelectedPlan<VM> as Plan>::Mutator;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start {
            self.start = false;
            VM::VMActivePlan::reset_mutator_iterator();
        }
        VM::VMActivePlan::get_next_mutator()
    }
}

pub trait ActivePlan<VM: VMBinding> {
    // TODO: I don't know how this can be implemented when we have multiple MMTk instances.
    // This function is used by space and phase to refer to the current plan.
    // Possibly we should remove the use of this function, and remove this function?
    fn global() -> &'static SelectedPlan<VM>;
    fn worker(tls: OpaquePointer) -> &'static mut GCWorker<VM>;
    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;
    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<VM> as Plan>::Mutator;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<VM> as Plan>::Mutator>;
    fn mutators<'a>() -> SynchronizedMutatorIterator<'a, VM> {
        SynchronizedMutatorIterator {
            _guard: Self::global().base().mutator_iterator_lock.lock().unwrap(),
            start: true,
            phantom: PhantomData,
        }
    }
    fn number_of_mutators() -> usize;
}
