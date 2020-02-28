use libc::c_void;
use mmtk::{Plan, SelectedPlan};
use mmtk::vm::ActivePlan;
use mmtk::util::OpaquePointer;
use OpenJDK;
use SINGLETON;
use super::UPCALLS;
use std::sync::Mutex;

pub struct VMActivePlan<> {}

impl ActivePlan<OpenJDK> for VMActivePlan {
    fn global() -> &'static SelectedPlan<OpenJDK> {
        &SINGLETON.plan
    }

    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::CollectorT {
        let c = ((*UPCALLS).active_collector)(tls);
        assert!(!c.is_null());
        unsafe { &mut *c }
    }

    unsafe fn is_mutator(tls: OpaquePointer) -> bool {
        ((*UPCALLS).is_mutator)(tls)
    }

    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT {
        let m = ((*UPCALLS).get_mmtk_mutator)(tls);
        unsafe { &mut *m }
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        unsafe {
            ((*UPCALLS).reset_mutator_iterator)();
        }
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT> {
        let _guard = MUTATOR_ITERATOR_LOCK.lock().unwrap();
        unsafe {
            let m = ((*UPCALLS).get_next_mutator)();
            if m.is_null() {
                None
            } else {
                Some(&mut *m)
            }
        }
    }
}

lazy_static! {
    pub static ref MUTATOR_ITERATOR_LOCK: Mutex<()> = Mutex::new(());
}
