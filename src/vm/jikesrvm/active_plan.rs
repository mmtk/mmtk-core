use ::vm::ActivePlan;
use ::plan::{Plan, SelectedPlan};
use ::util::Address;
use super::entrypoint::*;

pub struct VMActivePlan<> {}

impl<'a> ActivePlan<'a> for VMActivePlan {
    fn global() -> &'static SelectedPlan<'static> {
        &::plan::selected_plan::PLAN
    }

    fn collector(thread_id: usize) -> &'a <SelectedPlan<'a> as Plan>::CollectorT {
        unsafe {
            let thread = super::scheduling::VMScheduling::thread_from_id(thread_id);
            let system_thread = Address::from_usize(
                (thread + SYSTEM_THREAD_FIELD_OFFSET).load::<usize>());
            let cc = &*((system_thread + WORKER_INSTANCE_FIELD_OFFSET)
                .load::<*const <SelectedPlan as Plan>::CollectorT>());

            cc
        }
    }

    fn is_mutator(thread_id: usize) -> bool {
        unimplemented!()
    }

    fn mutator(thread_id: usize) -> &'a <SelectedPlan<'a> as Plan>::MutatorT {
        unimplemented!()
    }

    fn collector_count(thread_id: usize) -> usize {
        unimplemented!()
    }

    fn reset_mutator_iterator(thread_id: usize) {
        unimplemented!()
    }

    fn get_next_mutator(thread_id: usize) {
        unimplemented!()
    }
}