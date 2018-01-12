pub fn stop_all_mutators(thread_id: usize) {
    unimplemented!();
}

pub fn resume_mutators(thread_id: usize) {
    unimplemented!();
}

pub fn block_for_gc(thread_id: usize) {
    unimplemented!();
}

pub fn spawn_collector_thread<T: ParallelCollector>(ctx: &mut T) {
    unimplemented!();
}