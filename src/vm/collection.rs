use ::plan::{MutatorContext, ParallelCollector};

use libc::c_void;

pub trait Collection {
    fn stop_all_mutators(tls: *mut c_void);
    fn resume_mutators(tls: *mut c_void);
    fn block_for_gc(tls: *mut c_void);
    unsafe fn spawn_worker_thread<T: ParallelCollector>(tls: *mut c_void, ctx: *mut T);
    fn prepare_mutator<T: MutatorContext>(tls: *mut c_void, m: &T);
    fn out_of_memory(tls: *mut c_void) {
        panic!("Out of memory!");
    }
}