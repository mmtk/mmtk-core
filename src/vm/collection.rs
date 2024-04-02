use crate::util::alloc::AllocationError;
use crate::util::heap::gc_trigger::GCTriggerPolicy;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::{scheduler::*, Mutator};

/// Thread context for the spawned GC thread.  It is used by `spawn_gc_thread`.
/// Currently, `GCWorker` is the only kind of thread that mmtk-core will create.
pub enum GCThreadContext<VM: VMBinding> {
    /// The GC thread to spawn is a worker thread. There can be multiple worker threads.
    Worker(Box<GCWorker<VM>>),
}

/// VM-specific methods for garbage collection.
pub trait Collection<VM: VMBinding> {
    /// Stop all the mutator threads. MMTk calls this method when it requires all the mutator to yield for a GC.
    /// This method should not return until all the threads are yielded.
    /// The actual thread synchronization mechanism is up to the VM, and MMTk does not make assumptions on that.
    /// MMTk provides a callback function and expects the binding to use the callback for each mutator when it
    /// is ready for stack scanning. Usually a stack can be scanned as soon as the thread stops in the yieldpoint.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC worker.
    /// * `mutator_visitor`: A callback.  Call it with a mutator as argument to notify MMTk that the mutator is ready to be scanned.
    fn stop_all_mutators<F>(tls: VMWorkerThread, mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<VM>);

    /// Resume all the mutator threads, the opposite of the above. When a GC is finished, MMTk calls this method.
    ///
    /// This method may not be called by the same GC thread that called `stop_all_mutators`.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC worker.
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
    /// MMTk calls this method to spawn GC threads during [`crate::mmtk::MMTK::initialize_collection`]
    /// and [`crate::mmtk::MMTK::after_fork`].
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the parent thread that we spawn new threads from. This is the same `tls` when the VM
    ///   calls `initialize_collection()` and passes as an argument.
    /// * `ctx`: The context for the GC thread.
    ///   * If [`GCThreadContext::Worker`] is passed, it means spawning a thread to run as a GC worker.
    ///     The spawned thread shall call the entry point function `GCWorker::run`.
    ///     Currently `Worker` is the only kind of thread which mmtk-core will create.
    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<VM>);

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

    /// A hook for the VM to do work after forwarding objects.
    ///
    /// This function is called after all of the following have finished:
    /// -   The life and death of objects are determined.  Objects determined to be live will not
    ///     be reclaimed in this GC.
    /// -   Live objects have been moved to their destinations. (copying GC only)
    /// -   References in objects have been updated to point to new addresses. (copying GC only)
    ///
    /// And this function may run concurrently with the release work of GC, i.e. freeing the space
    /// occupied by dead objects.
    ///
    /// It is safe for the VM to read and write object fields at this time, although GC has not
    /// finished yet.  GC will be reclaiming spaces of dead objects, but will not damage live
    /// objects.  However, the VM cannot allocate new objects at this time.
    ///
    /// One possible use of this hook is enqueuing `{Soft,Weak,Phantom}Reference` instances to
    /// reference queues (for Java).  VMs (including JVM implementations) do not have to handle
    /// weak references this way, but mmtk-core provides this opportunity.
    ///
    /// Arguments:
    /// * `tls_worker`: The thread pointer for the worker thread performing this call.
    fn post_forwarding(_tls: VMWorkerThread) {}

    /// Return the amount of memory (in bytes) which the VM allocated outside the MMTk heap but
    /// wants to include into the current MMTk heap size.  MMTk core will consider the reported
    /// memory as part of MMTk heap for the purpose of heap size accounting.
    ///
    /// This amount should include memory that is kept alive by heap objects and can be released by
    /// executing finalizers (or other language-specific cleaning-up routines) that are executed
    /// when the heap objects are dead.  For example, if a language implementation allocates array
    /// headers in the MMTk heap, but allocates their underlying buffers that hold the actual
    /// elements using `malloc`, then those buffers should be included in this amount.  When the GC
    /// finds such an array dead, its finalizer shall `free` the buffer and reduce this amount.
    ///
    /// If possible, the VM should account off-heap memory in pages.  That is, count the number of
    /// pages occupied by off-heap objects, and report the number of bytes of those whole pages
    /// instead of individual objects.  Because the underlying operating system manages memory at
    /// page granularity, the occupied pages (instead of individual objects) determine the memory
    /// footprint of a process, and how much memory MMTk spaces can obtain from the OS.
    ///
    /// However, if the VM is incapable of accounting off-heap memory in pages (for example, if the
    /// VM uses `malloc` and the implementation of `malloc` is opaque to the VM), the VM binding
    /// can simply return the total number of bytes of those off-heap objects as an approximation.
    ///
    /// # Performance note
    ///
    /// This function will be called when MMTk polls for GC.  It happens every time the MMTk
    /// allocators have allocated a certain amount of memory, usually one or a few blocks.  Because
    /// this function is called very frequently, its implementation must be efficient.  If it is
    /// too expensive to compute the exact amount, an approximate value should be sufficient for
    /// MMTk to trigger GC promptly in order to release off-heap memory, and keep the memory
    /// footprint under control.
    fn vm_live_bytes() -> usize {
        // By default, MMTk assumes the amount of memory the VM allocates off-heap is negligible.
        0
    }

    /// Callback function to ask the VM whether GC is enabled or disabled, allowing or disallowing MMTk
    /// to trigger garbage collection. When collection is disabled, you can still allocate through MMTk,
    /// but MMTk will not trigger a GC even if the heap is full. In such a case, the allocation will
    /// exceed MMTk's heap size (the soft heap limit). However, there is no guarantee that the physical
    /// allocation will succeed, and if it succeeds, there is no guarantee that further allocation will
    /// keep succeeding. So if a VM disables collection, it needs to allocate with careful consideration
    /// to make sure that the physical memory allows the amount of allocation. We highly recommend
    /// to have GC always enabled (i.e. that this method always returns true). However, we support
    /// this to accomodate some VMs that require this behavior. Note that
    /// `handle_user_collection_request()` calls this function, too.  If this function returns
    /// false, `handle_user_collection_request()` will not trigger GC, either. Note also that any synchronization
    /// involving enabling and disabling collections by mutator threads should be implemented by the VM.
    fn is_collection_enabled() -> bool {
        // By default, MMTk assumes that collections are always enabled, and the binding should define
        // this method if the VM supports disabling GC, or if the VM cannot safely trigger GC until some
        // initialization is done, such as initializing class metadata for scanning objects.
        true
    }

    /// Ask the binding to create a [`GCTriggerPolicy`] if the option `gc_trigger` is set to
    /// `crate::util::options::GCTriggerSelector::Delegated`.
    fn create_gc_trigger() -> Box<dyn GCTriggerPolicy<VM>> {
        unimplemented!()
    }
}
