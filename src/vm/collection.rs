use crate::plan::MutatorContext;
use crate::scheduler::*;
use crate::scheduler::gc_works::ProcessEdgesWork;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::MMTK;

pub trait Collection<VM: VMBinding> {
    fn stop_all_mutators<E: ProcessEdgesWork<VM=VM>>(tls: OpaquePointer);
    fn resume_mutators(tls: OpaquePointer);
    fn block_for_gc(tls: OpaquePointer);
    fn spawn_worker_thread(tls: OpaquePointer, ctx: Option<&Worker<MMTK<VM>>>);
    fn prepare_mutator<T: MutatorContext<VM>>(tls: OpaquePointer, m: &T);
    fn out_of_memory(_tls: OpaquePointer) {
        panic!("Out of memory!");
    }
}
