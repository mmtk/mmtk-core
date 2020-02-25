use libc::c_void;
use mmtk::{Plan, SelectedPlan};
use mmtk::vm::ActivePlan;
use mmtk::util::OpaquePointer;
use OpenJDK;
use SINGLETON;

pub struct VMActivePlan<> {}

impl ActivePlan<OpenJDK> for VMActivePlan {
    fn global() -> &'static SelectedPlan<OpenJDK> {
        &SINGLETON.plan
    }

    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::CollectorT {
        unimplemented!()
    }

    unsafe fn is_mutator(tls: OpaquePointer) -> bool {
        // FIXME
        true
    }

    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT {
        unimplemented!()
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unimplemented!()
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT> {
        unimplemented!()
    }
}