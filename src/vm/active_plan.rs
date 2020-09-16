use crate::plan::{Plan, SelectedPlan};
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::scheduler::*;

pub trait ActivePlan<VM: VMBinding> {
    // TODO: I don't know how this can be implemented when we have multiple MMTk instances.
    // This function is used by space and phase to refer to the current plan.
    // Possibly we should remove the use of this function, and remove this function?
    fn global() -> &'static SelectedPlan<VM>;
    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    unsafe fn collector(
        tls: OpaquePointer,
    ) -> &'static mut <SelectedPlan<VM> as Plan>::CollectorT;
    unsafe fn worker(tls: OpaquePointer) -> &'static mut GCWorker<VM>;
    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;
    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<VM> as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<VM> as Plan>::MutatorT>;
}
