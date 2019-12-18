use ::plan::{Plan, SelectedPlan};
use ::util::OpaquePointer;
use libc::c_void;

pub trait ActivePlan {
    fn global() -> &'static SelectedPlan { &::mmtk::SINGLETON.plan }
    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan as Plan>::CollectorT;
    unsafe fn is_mutator(tls: OpaquePointer) -> bool;
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan as Plan>::MutatorT>;
}