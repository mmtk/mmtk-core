use ::plan::ParallelCollector;

pub trait Scheduling {
    fn stop_all_mutators(thread_id: usize);
    fn resume_mutators(thread_id: usize);
    fn block_for_gc(thread_id: usize);
    fn spawn_worker_thread<T: ParallelCollector>(thread_id: usize, ctx: &mut T);
}