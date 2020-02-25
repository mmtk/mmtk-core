use libc::c_void;

use mmtk::vm::Collection;
use mmtk::{MutatorContext, ParallelCollector};
use mmtk::util::OpaquePointer;

use OpenJDK;
use UPCALLS;

pub struct VMCollection {}

impl Collection<OpenJDK> for VMCollection {
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

    unsafe fn spawn_worker_thread<T: ParallelCollector<OpenJDK>>(tls: OpaquePointer, ctx: *mut T) {
        unimplemented!();
    }

    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T) {
        unimplemented!()
    }
}