use super::super::Collection;
use ::plan::{MutatorContext, ParallelCollector};

pub struct VMCollection {}

impl Collection for VMCollection {
    fn stop_all_mutators(thread_id: usize) {
        unimplemented!();
    }

    fn resume_mutators(thread_id: usize) {
        unimplemented!();
    }

    fn block_for_gc(thread_id: usize) {
        unimplemented!();
    }

    unsafe fn spawn_worker_thread<T: ParallelCollector>(thread_id: usize, ctx: *mut T) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(thread_id: usize, m: &T) {
        unimplemented!()
    }
}