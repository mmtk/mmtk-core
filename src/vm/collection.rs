use crate::plan::{MutatorContext, ParallelCollector};
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub trait Collection<VM: VMBinding> {
    fn stop_all_mutators(tls: OpaquePointer);
    fn resume_mutators(tls: OpaquePointer);
    fn block_for_gc(tls: OpaquePointer);
    fn spawn_worker_thread<T: ParallelCollector<VM>>(tls: OpaquePointer, ctx: Option<&mut T>);
    fn prepare_mutator<T: MutatorContext>(tls: OpaquePointer, m: &T);
    fn out_of_memory(_tls: OpaquePointer) {
        panic!("Out of memory!");
    }
}
