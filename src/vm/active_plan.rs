use ::plan::{Plan, SelectedPlan};
use ::util::OpaquePointer;
use libc::c_void;

pub trait ActivePlan {
    // TODO: This function should not be defined in the trait. If we have multiple instances of MMTk,
    //    only the VM can tell us which instance we are using.
    fn global() -> &'static SelectedPlan { &::mmtk::SINGLETON.plan }
    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan as Plan>::CollectorT;
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan as Plan>::MutatorT>;
}