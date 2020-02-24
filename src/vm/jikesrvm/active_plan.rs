use ::vm::ActivePlan;
use ::plan::{Plan, SelectedPlan};
use ::util::{Address, SynchronizedCounter};
use ::util::OpaquePointer;

use super::entrypoint::*;
use super::collection::VMCollection;
use super::JTOC_BASE;

use std::mem;
use libc::c_void;
use vm::jikesrvm::JikesRVM;

static MUTATOR_COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMActivePlan<> {}

impl ActivePlan<JikesRVM> for VMActivePlan {
    fn global() -> &'static SelectedPlan<JikesRVM> {
        &::mmtk::SINGLETON.plan
    }

    // XXX: Are they actually static
    unsafe fn collector(tls: OpaquePointer) -> &'static mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::CollectorT {
        let thread: Address = unsafe { mem::transmute(tls) };
        let system_thread = Address::from_usize(
            (thread + SYSTEM_THREAD_FIELD_OFFSET).load::<usize>());
        let cc = &mut *((system_thread + WORKER_INSTANCE_FIELD_OFFSET)
            .load::<*mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::CollectorT>());

        cc
    }

    unsafe fn is_mutator(tls: OpaquePointer) -> bool {
        let thread: Address = unsafe { mem::transmute(tls) };
        !(thread + IS_COLLECTOR_FIELD_OFFSET).load::<bool>()
    }

    // XXX: Are they actually static
    unsafe fn mutator(tls: OpaquePointer) -> &'static mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::MutatorT {
        let thread: Address = unsafe { mem::transmute(tls) };
        let mutator = (thread + MMTK_HANDLE_FIELD_OFFSET).load::<usize>();
        &mut *(mutator as *mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::MutatorT)
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        MUTATOR_COUNTER.reset();
    }

    fn get_next_mutator() -> Option<&'static mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::MutatorT> {
        loop {
            let idx = MUTATOR_COUNTER.increment();
            let num_threads = unsafe { (JTOC_BASE + NUM_THREADS_FIELD_OFFSET).load::<usize>() };
            if idx >= num_threads {
                return None;
            } else {
                let t = unsafe { VMCollection::thread_from_index(idx) };
                let active_mutator_context = unsafe { (t + ACTIVE_MUTATOR_CONTEXT_FIELD_OFFSET)
                    .load::<bool>() };
                if active_mutator_context {
                    unsafe {
                        let mutator = (t + MMTK_HANDLE_FIELD_OFFSET).load::<usize>();
                        let ret =
                            &mut *(mutator as *mut <SelectedPlan<JikesRVM> as Plan<JikesRVM>>::MutatorT);
                        return Some(ret);
                    }
                }
            }
        }
    }
}