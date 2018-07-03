use ::plan::{MutatorContext, ParallelCollector};

pub trait Collection {
    fn stop_all_mutators(thread_id: usize);
    fn resume_mutators(thread_id: usize);
    fn block_for_gc(thread_id: usize);
    unsafe fn spawn_worker_thread<T: ParallelCollector>(thread_id: usize, ctx: *mut T);
    fn prepare_mutator<T: MutatorContext>(thread_id: usize, m: &T);
    fn out_of_memory(thread_id: usize) {
        panic!("Out of memory!");
    }
}