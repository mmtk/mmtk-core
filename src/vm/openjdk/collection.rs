use super::super::Collection;
use ::plan::{MutatorContext, ParallelCollector};
use ::util::OpaquePointer;

use super::UPCALLS;

use libc::c_void;

pub struct VMCollection {}

impl Collection for VMCollection {
    fn stop_all_mutators(tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).stop_all_mutators)(tls);
        }
    }

    fn resume_mutators(tls: OpaquePointer) {
        unsafe {
            ((*UPCALLS).resume_mutators)(tls);
        }
    }

    fn block_for_gc(tls: OpaquePointer) {
        unimplemented!();
    }

    unsafe fn spawn_worker_thread<T: ParallelCollector>(tls: OpaquePointer, ctx: *mut T) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T) {
        unimplemented!()
    }
}