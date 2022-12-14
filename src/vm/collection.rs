use crate::plan::MutatorContext;
use crate::util::alloc::AllocationError;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::{scheduler::*, Mutator};

use super::scanning::QueuingTracerFactory;

/// Thread context for the spawned GC thread.  It is used by spawn_gc_thread.
pub enum GCThreadContext<VM: VMBinding> {
    Controller(Box<GCController<VM>>),
    Worker(Box<GCWorker<VM>>),
}

/// Information related to weak reference processing.  Used by `Collection::process_weak_refs`.
pub struct ProcessWeakRefsContext {
    /// `true` if `process_weak_refs` is called during the forwarding phase in MarkCompact.
    /// Always `false` if the GC is not MarkCompact.
    pub forwarding: bool,

    /// `true` if the current GC is a nursery GC.
    /// Always `false` if not using a generationl GC algorithm.
    pub nursery: bool,
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

    /// Allow VM-specific behaviors after all the mutators are stopped and before any actual GC
    /// work (including root scanning) starts.
    ///
    /// Arguments:
    /// * `tls_worker`: The thread pointer for the worker thread performing this call.
    fn vm_prepare(_tls: VMWorkerThread) {}

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

    /// Process weak references.
    ///
    /// This function is called after a transitive closure is completed.
    ///
    /// MMTk core enables the VM binding to do the following in this function:
    ///
    /// 1.  Query if an object is already reached in this transitive closure.
    /// 2.  Keep certain objects and their descendents alive.
    /// 3.  Get the new address of objects that are either
    ///     -   already alive before this function is called, or
    ///     -   explicitly kept alive in this function.
    /// 4.  Request this function to be called again after transitive closure is finished again.
    ///
    /// The VM binding can call `ObjectReference::is_reachable()` to query if an object is
    /// currently reached.
    ///
    /// The VM binding can use `tracer_factory` to get access to an `ObjectTracer`, and call
    /// its `trace_object(object)` method to keep `object` and its decendents alive.
    ///
    /// The return value of `ObjectTracer::trace_object(object)` is the new address of the given
    /// `object` if it is moved by the GC.
    ///
    /// The VM binding can return `true` from `process_weak_refs` to request `process_weak_refs`
    /// to be called again after the MMTk core finishes transitive closure again from the objects
    /// newly visited by `ObjectTracer::trace_object`.  This is useful if a VM supports multiple
    /// levels of reachabilities (such as Java) or ephemerons.
    ///
    /// Implementation-wise, this function is called as the "sentinel" of the `VMRefClosure` work
    /// bucket, which means it is called when all work packets in that bucket have finished.  The
    /// `tracer_factory` expands the transitive closure by adding more work packets in the same
    /// bucket.  This means if `process_weak_refs` returns true, those work packets will have
    /// finished (completing the transitive closure) by the time `process_weak_refs` is called
    /// again.  The VM binding can make use of this by adding custom work packets into the
    /// `VMRefClosure` bucket.  The bucket will be `VMRefForwarding`, instead, when forwarding.
    /// See below.
    ///
    /// GC algorithms other than mark-compact compute transitive closure only once.  Mark-compact
    /// GC will compute transive closure twice during each GC.  It will mark objects in the first
    /// transitive closure, and forward references in the second transitive closure. During the
    /// second transitive closure, `context.forwarding` will be `true`.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `context`: Provides more information of the current trace.
    /// * `tracer_factory`: Use this to create an `ObjectTracer` and use it to retain and update
    ///   weak references.
    ///
    /// This function shall return true if this function needs to be called again after the GC
    /// finishes expanding the transitive closure from the objects kept alive.
    fn process_weak_refs(
        _worker: &mut GCWorker<VM>,
        _context: ProcessWeakRefsContext,
        _tracer_factory: impl QueuingTracerFactory<VM>,
    ) -> bool {
        false
    }
}
