use ::plan::{MutatorContext, ParallelCollector};
use ::util::OpaquePointer;

use libc::c_void;

pub trait Collection {
    fn stop_all_mutators(tls: OpaquePointer);
    fn resume_mutators(tls: OpaquePointer);
    fn block_for_gc(tls: OpaquePointer);
    unsafe fn spawn_worker_thread<T: ParallelCollector>(tls: OpaquePointer, ctx: *mut T);
    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T);
    fn out_of_memory(tls: OpaquePointer) {
        panic!("Out of memory!");
    }
}