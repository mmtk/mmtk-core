use ::plan::{Plan, SelectedPlan};

use libc::c_void;

pub trait ActivePlan {
    fn global() -> &'static SelectedPlan { &::mmtk::SINGLETON.plan }
    unsafe fn collector(tls: *mut c_void) -> &'static mut <SelectedPlan as Plan>::CollectorT;
    unsafe fn is_mutator(tls: *mut c_void) -> bool;
    unsafe fn mutator(tls: *mut c_void) -> &'static mut <SelectedPlan as Plan>::MutatorT;
    fn collector_count() -> usize;
    fn reset_mutator_iterator();
    fn get_next_mutator() -> Option<&'static mut <SelectedPlan as Plan>::MutatorT>;
}