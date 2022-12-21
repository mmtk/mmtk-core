use crate::plan::MutatorContext;
use crate::util::alloc::AllocationError;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::{scheduler::*, Mutator};

/// Thread context for the spawned GC thread.  It is used by spawn_gc_thread.
pub enum GCThreadContext<VM: VMBinding> {
    Controller(Box<GCController<VM>>),
    Worker(Box<GCWorker<VM>>),
}

/// VM-specific methods for garbage collection.
pub trait Collection<VM: VMBinding> {
    /// If true, only the coordinator thread can call stop_all_mutators and the resume_mutators methods.
    /// If false, any GC thread can call these methods.
    ///
    /// This constant exists because some VMs require the thread that resumes a thread to be the same thread that
    /// stopped it.  The MMTk Core will use the appropriate thread to stop or start the world according to the value of
    /// this constant.  If a VM does not have such a requirement, the VM binding shall set this to false to reduce an
    /// unnecessary context switch.
    const COORDINATOR_ONLY_STW: bool = true;

    /// Stop all the mutator threads. MMTk calls this method when it requires all the mutator to yield for a GC.
    /// This method is called by a single thread in MMTk (the GC controller).
    /// This method should not return until all the threads are yielded.
    /// The actual thread synchronization mechanism is up to the VM, and MMTk does not make assumptions on that.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC controller/coordinator.
    fn stop_all_mutators<F>(tls: VMWorkerThread, mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<VM>);

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
    /// MMTk calls this method to spawn GC threads during [`initialize_collection()`](../memory_manager/fn.initialize_collection.html).
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the parent thread that we spawn new threads from. This is the same `tls` when the VM
    ///   calls `initialize_collection()` and passes as an argument.
    /// * `ctx`: The context for the GC thread.
    ///   * If `Controller` is passed, it means spawning a thread to run as the GC controller.
    ///     The spawned thread shall call `memory_manager::start_control_collector`.
    ///   * If `Worker` is passed, it means spawning a thread to run as a GC worker.
    ///     The spawned thread shall call `memory_manager::start_worker`.
    ///   In either case, the `Box` inside should be passed back to the called function.
    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<VM>);

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

    /// Inform the VM of an out-of-memory error. The binding should hook into the VM's error
    /// routine for OOM. Note that there are two different categories of OOM:
    ///  * Critical OOM: This is the case where the OS is unable to mmap or acquire more memory.
    ///    MMTk expects the VM to abort immediately if such an error is thrown.
    ///  * Heap OOM: This is the case where the specified heap size is insufficient to execute the
    ///    application. MMTk expects the binding to notify the VM about this OOM. MMTk makes no
    ///    assumptions about whether the VM will continue executing or abort immediately.
    ///
    /// See [`AllocationError`] for more information.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the mutator which failed the allocation and triggered the OOM.
    /// * `err_kind`: The type of OOM error that was encountered.
    fn out_of_memory(_tls: VMThread, err_kind: AllocationError) {
        panic!("Out of memory with {:?}!", err_kind);
    }

    /// Inform the VM to schedule finalization threads.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the current GC thread.
    fn schedule_finalization(_tls: VMWorkerThread) {}

    /// Inform the VM to do its VM-specific release work at the end of a GC.
    ///
    /// Arguments:
    /// * `tls_worker`: The thread pointer for the worker thread performing this call.
    fn vm_release(_tls: VMWorkerThread) {}
}
