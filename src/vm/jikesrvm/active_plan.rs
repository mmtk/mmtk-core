use ::vm::ActivePlan;
use ::plan::{Plan, SelectedPlan};
use ::util::{Address, SynchronizedCounter};
use super::entrypoint::*;
use super::scheduling::VMScheduling;
use super::JTOC_BASE;

static MUTATOR_COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMActivePlan<> {}

impl<'a> ActivePlan<'a> for VMActivePlan {
    fn global() -> &'static SelectedPlan<'static> {
        &::plan::selected_plan::PLAN
    }

    unsafe fn collector(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::CollectorT {
        let thread = VMScheduling::thread_from_id(thread_id);
        let system_thread = Address::from_usize(
            (thread + SYSTEM_THREAD_FIELD_OFFSET).load::<usize>());
        let cc = &mut *((system_thread + WORKER_INSTANCE_FIELD_OFFSET)
            .load::<*mut <SelectedPlan as Plan>::CollectorT>());

        cc
    }

    unsafe fn is_mutator(thread_id: usize) -> bool {
        let thread = VMScheduling::thread_from_id(thread_id);
        !(thread + IS_COLLECTOR_FIELD_OFFSET).load::<bool>()
    }

    unsafe fn mutator(thread_id: usize) -> &'a mut <SelectedPlan<'a> as Plan>::MutatorT {
        &mut *(VMScheduling::thread_from_id(thread_id).as_usize()
            as *mut <SelectedPlan<'a> as Plan>::MutatorT)
    }

    fn collector_count() -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator() {
        MUTATOR_COUNTER.reset();
    }

    fn get_next_mutator() -> Option<&'a mut <SelectedPlan<'a> as Plan>::MutatorT> {
        loop {
            let idx = MUTATOR_COUNTER.increment();
            let num_threads = unsafe { (JTOC_BASE + NUM_THREADS_FIELD_OFFSET).load::<usize>() };
            if idx >= num_threads {
                return None;
            } else {
                let t = unsafe { VMScheduling::thread_from_index(idx) };
                let active_mutator_context = unsafe { (t + ACTIVE_MUTATOR_CONTEXT_FIELD_OFFSET)
                    .load::<bool>() };
                if active_mutator_context {
                    let ret = unsafe {
                        &mut *(t.as_usize() as *mut <SelectedPlan<'a> as Plan>::MutatorT)
                    };
                    return Some(ret);
                }
            }
        }
    }
}