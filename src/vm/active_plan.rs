use crate::plan::{Plan, SelectedPlan};
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub trait ActivePlan<VM: VMBinding> {
    // TODO: I don't know how this can be implemented when we have multiple MMTk instances.
    // This function is used by space and phase to refer to the current plan.
    // Possibly we should remove the use of this function, and remove this function?
    fn global() -> &'static SelectedPlan<VM>;
    // FIXME: I am not sure why this is unsafe.
    #[allow(clippy::missing_safety_doc)]
    unsafe fn collector(
        tls: OpaquePointer,
    ) -> &'static mut <SelectedPlan<VM> as Plan<VM>>::CollectorT;
    // FIXME: I am not sure why this is unsafe.
    #[allow(clippy::missing_safety_doc)]
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;
    // FIXME: I am not sure why this is unsafe.
    #[allow(clippy::missing_safety_doc)]
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<VM> as Plan<VM>>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<VM> as Plan<VM>>::MutatorT>;
}
