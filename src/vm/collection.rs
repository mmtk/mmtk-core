use crate::plan::MutatorContext;
use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::*;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;

/// VM-specific methods for garbage collection.
pub trait Collection<VM: VMBinding> {
    /// Stop all the mutator threads. MMTk calls this method when it requires all the mutator to yield for a GC.
    /// This method is called by a single thread in MMTk (the GC controller).
    /// This method should not return until all the threads are yielded.
    /// The actual thread synchronization mechanism is up to the VM, and MMTk does not make assumptions on that.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC controller/coordinator.
    fn stop_all_mutators<E: ProcessEdgesWork<VM = VM>>(tls: VMWorkerThread);

    /// Resume all the mutator threads, the opposite of the above. When a GC is finished, MMTk calls this method.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC controller/coordinator.
    fn resume_mutators(tls: VMWorkerThread);

    /// Block the current thread for GC. This is called when an allocation request cannot be fulfilled and a GC
    /// is needed. MMTk calls this method to inform the VM that the current thread needs to be blocked as a GC
    /// is going to happen. Then MMTk starts a GC. For a stop-the-world GC, MMTk will then call `stop_all_mutators()`
    /// before the GC, and call `resume_mutators()` after the GC.
    ///
    /// Arguments:
    /// * `tls`: The current thread pointer that should be blocked. The VM can optionally check if the current thread matches `tls`.
    fn block_for_gc(tls: VMMutatorThread);

    /// Ask the VM to spawn a GC thread for MMTk. A GC thread may later call into the VM through these VM traits. Some VMs
    /// have assumptions that those calls needs to be within VM internal threads.
    /// As a result, MMTk does not spawn GC threads itself to avoid breaking this kind of assumptions.
    /// MMTk calls this method to spawn GC threads during [`enable_collection()`](../memory_manager/fn.enable_collection.html).
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the parent thread that we spawn new threads from. This is the same `tls` when the VM
    ///   calls `enable_collection()` and passes as an argument.
    /// * `ctx`: The GC worker context for the GC thread. If `None` is passed, it means spawning a GC thread for the GC controller,
    ///   which does not have a worker context.
    fn spawn_worker_thread(tls: VMThread, ctx: Option<&GCWorker<VM>>);

    /// Allow VM-specific behaviors for a mutator after all the mutators are stopped and before any actual GC work starts.
    ///
    /// Arguments:
    /// * `tls_worker`: The thread pointer for the worker thread performing this call.
    /// * `tls_mutator`: The thread pointer for the target mutator thread.
    /// * `m`: The mutator context for the thread.
    fn prepare_mutator<T: MutatorContext<VM>>(
        tls_worker: VMWorkerThread,
        tls_mutator: VMMutatorThread,
        m: &T,
    );

    /// Inform the VM for an out-of-memory error. The VM can implement its own error routine for OOM.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the mutator which failed the allocation and triggered the OOM.
    fn out_of_memory(_tls: VMThread) {
        panic!("Out of memory!");
    }

    /// Inform the VM to schedule finalization threads.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the current GC thread.
    fn schedule_finalization(_tls: VMWorkerThread) {}
}
