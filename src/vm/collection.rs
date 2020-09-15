use crate::plan::{MutatorContext, ParallelCollector};
use crate::scheduler::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub trait Collection<VM: VMBinding> {
    fn stop_all_mutators(tls: OpaquePointer);
    fn resume_mutators(tls: OpaquePointer);
    fn block_for_gc(tls: OpaquePointer);
    fn spawn_worker_thread(tls: OpaquePointer, ctx: Option<&Worker<VM>>);
    fn prepare_mutator<T: MutatorContext<VM>>(tls: OpaquePointer, m: &T);
    fn out_of_memory(_tls: OpaquePointer) {
        panic!("Out of memory!");
    }
}
