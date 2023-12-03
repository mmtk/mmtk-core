use super::prelude::*;

/// Default min alignment 4 bytes
const DEFAULT_LOG_MIN_ALIGNMENT: usize = 2;
/// Default max alignment 8 bytes
const DEFAULT_LOG_MAX_ALIGNMENT: usize = 3;

/// Thread context for the spawned GC thread.  It is used by spawn_gc_thread.
pub enum GCThreadContext<VM: VMBinding> {
    /// The GC thread to spawn is a controller thread. There is only one controller thread.
    Controller(Box<GCController<VM>>),
    /// The GC thread to spawn is a worker thread. There can be multiple worker threads.
    Worker(Box<GCWorker<VM>>),
}

/// The `VMBinding` trait associates with each trait, and provides VM-specific constants.
pub trait VMBinding
where
    Self: Sized + 'static + Send + Sync + Default,
{
    /// The type of edges in this VM.
    type VMEdge: edge_shape::Edge;
    /// The type of heap memory slice in this VM.
    type VMMemorySlice: edge_shape::MemorySlice<Edge = Self::VMEdge>;

    /// A value to fill in alignment gaps. This value can be used for debugging.
    const ALIGNMENT_VALUE: usize = 0xdead_beef;
    /// Allowed minimal alignment in bytes.
    const MIN_ALIGNMENT: usize = 1 << DEFAULT_LOG_MIN_ALIGNMENT;
    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = 1 << DEFAULT_LOG_MAX_ALIGNMENT;
    /// Does the binding use a non-zero allocation offset? If this is false, we expect the binding
    /// to always use offset === 0 for allocation, and we are able to do some optimization if we know
    /// offset === 0.
    const USE_ALLOCATION_OFFSET: bool = true;

    /// This value is used to assert if the cursor is reasonable after allocations.
    /// At the end of an allocation, the allocation cursor should be aligned to this value.
    /// Note that MMTk does not attempt to do anything to align the cursor to this value, but
    /// it merely asserts with this constant.
    const ALLOC_END_ALIGNMENT: usize = 1;

    // --- Active Plan ---

    /// Return whether there is a mutator created and associated with the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is valid (a value passed in by the VM binding through API).
    fn is_mutator(tls: VMThread) -> bool;

    /// Return a `Mutator` reference for the thread.
    ///
    /// Arguments:
    /// * `tls`: The thread to query.
    ///
    /// # Safety
    /// The caller needs to make sure that the thread is a mutator thread.
    fn mutator(tls: VMMutatorThread) -> &'static mut Mutator<Self>;

    /// Return an iterator that includes all the mutators at the point of invocation.
    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<Self>> + 'a>;

    /// Return the total count of mutators.
    fn number_of_mutators() -> usize;

    /// The fallback for object tracing. MMTk generally expects to find an object in one of MMTk's spaces (if it is allocated by MMTK),
    /// and apply the corresponding policy to trace the object. Tracing in MMTk means identifying whether we have encountered this object in the
    /// current GC. For example, for mark sweep, we will check if an object is marked, and if it is not yet marked, mark and enqueue the object
    /// for later scanning. For copying policies, copying also happens in this step. For example for MMTk's copying space, we will
    /// copy an object if it is in 'from space', and enqueue the copied object for later scanning.
    ///
    /// If a binding would like to trace objects that are not allocated by MMTk and are not in any MMTk space, they can override this method.
    /// They should check whether the object is encountered before in this current GC. If not, they should record the object as encountered themselves,
    /// and enqueue the object reference to the object queue provided by the argument. If a binding moves objects, they should do the copying in the method,
    /// and enqueue the new object reference instead.
    ///
    /// The method should return the new object reference if the method moves the object, otherwise return the original object reference.
    ///
    /// Arguments:
    /// * `queue`: The object queue. If an object is encountered for the first time in this GC, we expect the implementation to call `queue.enqueue()`
    ///            for the object. If the object is moved during the tracing, the new object reference (after copying) should be enqueued instead.
    /// * `object`: The object to trace.
    /// * `worker`: The GC worker that is doing this tracing. This is used to copy object (see [`crate::vm::VMBinding::copy_object`])
    fn vm_trace_object<Q: ObjectQueue>(
        _queue: &mut Q,
        object: ObjectReference,
        _worker: &mut GCWorker<Self>,
    ) -> ObjectReference {
        panic!("MMTk cannot trace object {:?} as it does not belong to any MMTk space. If the object is known to the VM, the binding can override this method and handle its tracing.", object)
    }

    // --- Collection ---

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
        F: FnMut(&'static mut Mutator<Self>);

    /// Resume all the mutator threads, the opposite of the above. When a GC is finished, MMTk calls this method.
    ///
    /// This method may not be called by the same GC thread that called `stop_all_mutators`.
    ///
    /// Arguments:
    /// * `tls`: The thread pointer for the GC worker.  Currently it is the tls of the embedded `GCWorker` instance
    /// of the coordinator thread, but it is subject to change, and should not be depended on.
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
    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<Self>);

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

    // --- Object Model ---

    // Per-object Metadata Spec definitions go here
    //
    // Note a number of Global and PolicySpecific side metadata specifications are already reserved by mmtk-core.
    // Any side metadata offset calculation must consider these to prevent overlaps. A binding should start their
    // side metadata from GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS or LOCAL_SIDE_METADATA_VM_BASE_ADDRESS.

    /// A global 1-bit metadata used by generational plans to track cross-generational pointers. It is generally
    /// located in side metadata.
    ///
    /// Note that for this bit, 0 represents logged (default), and 1 represents unlogged.
    /// This bit is also referred to as unlogged bit in Java MMTk for this reason.
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec;

    /// A local word-size metadata for the forwarding pointer, used by copying plans. It is almost always
    /// located in the object header as it is fine to destroy an object header in order to copy it.
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec;

    /// A local 2-bit metadata for the forwarding status bits, used by copying plans. If your runtime requires
    /// word-aligned addresses (i.e. 4- or 8-bytes), you can use the last two bits in the object header to store
    /// the forwarding bits. Note that you must be careful if you place this in the header as the runtime may
    /// be using those bits for some other reason.
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec;

    /// A local 1-bit metadata for the mark bit, used by most plans that need to mark live objects. Like with the
    /// [forwarding bits](crate::vm::VMBinding::LOCAL_FORWARDING_BITS_SPEC), you can often steal the last bit in
    /// the object header (due to alignment requirements) for the mark bit. Though some bindings such as the
    /// OpenJDK binding prefer to have the mark bits in side metadata to allow for bulk operations.
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec;

    #[cfg(feature = "object_pinning")]
    /// A local 1-bit metadata specification for the pinning bit, used by plans that need to pin objects. It is
    /// generally in side metadata.
    const LOCAL_PINNING_BIT_SPEC: VMLocalPinningBitSpec;

    /// A local 2-bit metadata used by the large object space to mark objects and set objects as "newly allocated".
    /// Used by any plan with large object allocation. It is generally in the header as we can add an extra word
    /// before the large object to store this metadata. This is fine as the metadata size is insignificant in
    /// comparison to the object size.
    //
    // TODO: Cleanup and place the LOS mark and nursery bits in the header. See here: https://github.com/mmtk/mmtk-core/issues/847
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec;

    /// Set this to true if the VM binding requires the valid object (VO) bits to be available
    /// during tracing. If this constant is set to `false`, it is undefined behavior if the binding
    /// attempts to access VO bits during tracing.
    ///
    /// Note that the VO bits is always available during root scanning even if this flag is false,
    /// which is suitable for using VO bits (and the `is_mmtk_object()` method) for conservative
    /// stack scanning. However, if a binding is also conservative in finding references during
    /// object scanning, they need to set this constant to `true`. See the comments of individual
    /// methods in the `Scanning` trait.
    ///
    /// Depending on the internal implementation of mmtk-core, different strategies for handling
    /// VO bits have different time/space overhead.  mmtk-core will choose the best strategy
    /// according to the configuration of the VM binding, including this flag.  Currently, setting
    /// this flag to true does not impose any additional overhead.
    #[cfg(feature = "vo_bit")]
    const NEED_VO_BITS_DURING_TRACING: bool = false;

    /// A function to non-atomically load the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic load, thus not thread-safe.
    unsafe fn load_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        mask: Option<T>,
    ) -> T {
        metadata_spec.load::<T>(object.to_header::<Self>(), mask)
    }

    /// A function to atomically load the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is the atomic ordering for the load operation.
    fn load_metadata_atomic<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        mask: Option<T>,
        ordering: Ordering,
    ) -> T {
        metadata_spec.load_atomic::<T>(object.to_header::<Self>(), mask, ordering)
    }

    /// A function to non-atomically store a value to the specified per-object metadata.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic store, thus not thread-safe.
    unsafe fn store_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
    ) {
        metadata_spec.store::<T>(object.to_header::<Self>(), val, mask)
    }

    /// A function to atomically store a value to the specified per-object metadata.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is the optional atomic ordering for the store operation.
    fn store_metadata_atomic<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
        ordering: Ordering,
    ) {
        metadata_spec.store_atomic::<T>(object.to_header::<Self>(), val, mask, ordering)
    }

    /// A function to atomically compare-and-exchange the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns `true` if the operation is successful, and `false` otherwise.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `old_val`: is the expected current value of the metadata.
    /// * `new_val`: is the new metadata value to be stored if the compare-and-exchange operation is successful.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `success_order`: is the atomic ordering used if the operation is successful.
    /// * `failure_order`: is the atomic ordering used if the operation fails.
    fn compare_exchange_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        old_val: T,
        new_val: T,
        mask: Option<T>,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> std::result::Result<T, T> {
        metadata_spec.compare_exchange::<T>(
            object.to_header::<Self>(),
            old_val,
            new_val,
            mask,
            success_order,
            failure_order,
        )
    }

    /// A function to atomically perform an add operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// This is a wrapping add.
    /// # Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be added to the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_add_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_add::<T>(object.to_header::<Self>(), val, order)
    }

    /// A function to atomically perform a subtract operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// This is a wrapping sub.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be subtracted from the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_sub_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_sub::<T>(object.to_header::<Self>(), val, order)
    }

    /// A function to atomically perform a bit-and operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-and with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_and_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_and::<T>(object.to_header::<Self>(), val, order)
    }

    /// A function to atomically perform a bit-or operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-or with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_or_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_or::<T>(object.to_header::<Self>(), val, order)
    }

    /// A function to atomically perform an update operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// The semantics of this method are the same as the `fetch_update()` on Rust atomic types.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-and with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    /// # Returns the old metadata value.
    fn fetch_update_metadata<T: MetadataValue, F: FnMut(T) -> Option<T> + Copy>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> std::result::Result<T, T> {
        metadata_spec.fetch_update::<T, F>(object.to_header::<Self>(), set_order, fetch_order, f)
    }

    /// Copy an object and return the address of the new object. Usually in the implementation of this method,
    /// `alloc_copy()` and `post_copy()` from [`GCWorkerCopyContext`](util/copy/struct.GCWorkerCopyContext.html)
    /// are used for copying.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `semantics`: The copy semantic to use.
    /// * `copy_context`: The `GCWorkerCopyContext` for the GC thread.
    fn copy_object(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<Self>,
    ) -> ObjectReference;

    /// Copy an object. This is required
    /// for delayed-copy collectors such as compacting collectors. During the
    /// collection, MMTk reserves a region in the heap for an object as per
    /// requirements found from `ObjectModel` and then asks `ObjectModel` to
    /// determine what the object's reference will be post-copy. Return the address
    /// past the end of the copied object.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `to`: The target location.
    /// * `region: The start of the region that was reserved for this object.
    fn copy_object_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address;

    /// Return the reference that an object will be referred to after it is copied
    /// to the specified region. Used in delayed-copy collectors such as compacting
    /// collectors.
    ///
    /// Arguments:
    /// * `from`: The object to be copied.
    /// * `to`: The region to be copied to.
    fn get_object_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference;

    /// Return the size used by an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_object_size(object: ObjectReference) -> usize;

    /// Return the size when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_object_size_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_object_align_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment offset when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_object_align_offset_when_copied(object: ObjectReference) -> usize;

    /// This is the worst case expansion that can occur due to object size increasing while
    /// copying. This constant is used to calculate whether a nursery has grown larger than the
    /// mature space for generational plans.
    const VM_WORST_CASE_COPY_EXPANSION: f64 = 1.5;

    /// If this is true, the binding guarantees that an object reference's raw address is always equal to the return value of the `ref_to_address` method
    /// and the return value of the `ref_to_object_start` method. This is a very strong guarantee, but it is also helpful for MMTk to
    /// make some assumptions and optimize for this case.
    /// If a binding sets this to true, and the related methods return inconsistent results, this is an undefined behavior. MMTk may panic
    /// if any assertion catches this error, but may also fail silently.
    const UNIFIED_OBJECT_REFERENCE_ADDRESS: bool = false;

    /// For our allocation result (object_start), the binding may have an offset between the allocation result
    /// and the raw address of their object reference, i.e. object ref's raw address = object_start + offset.
    /// The offset could be zero. The offset is not necessary to be
    /// constant for all the objects. This constant defines the smallest possible offset.
    ///
    /// This is used as an indication for MMTk to predict where object references may point to in some algorithms.
    ///
    /// We should have the invariant:
    /// * object ref >= object_start + OBJECT_REF_OFFSET_LOWER_BOUND
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize;

    /// Return the lowest address of the storage associated with an object. This should be
    /// the address that a binding gets by an allocation call ([`crate::memory_manager::alloc`]).
    ///
    /// Arguments:
    /// * `object`: The object to be queried. It should not be null.
    fn ref_to_object_start(object: ObjectReference) -> Address;

    /// Return the header base address from an object reference. Any object header metadata
    /// in the [`crate::vm::VMBinding`] declares a piece of header metadata with an offset
    /// from this address. If a binding does not use any header metadata for MMTk, this method
    /// will not be called, and the binding can simply use `unreachable!()` for the method.
    ///
    /// Arguments:
    /// * `object`: The object to be queried. It should not be null.
    fn ref_to_header(object: ObjectReference) -> Address;

    /// Return an address guaranteed to be inside the storage associated
    /// with an object. The returned address needs to be deterministic
    /// for an given object. For a given object, the returned address
    /// should be a constant offset from the object reference address.
    ///
    /// Note that MMTk may forge an arbitrary address
    /// directly into a potential object reference, and call this method on the 'object reference'.
    /// In that case, the argument `object` may not be a valid object reference,
    /// and the implementation of this method should not use any object metadata.
    ///
    /// MMTk uses this method more frequently than [`crate::vm::VMBinding::ref_to_object_start`].
    ///
    /// Arguments:
    /// * `object`: The object to be queried. It should not be null.
    fn ref_to_address(object: ObjectReference) -> Address;

    /// Return an object for a given address returned by `ref_to_address()`.
    /// This does exactly the opposite of `ref_to_address()`. The argument `addr` has
    /// to be an address that is previously returned from `ref_to_address()`. Invoking this method
    /// with an unexpected address is undefined behavior.
    ///
    /// Arguments:
    /// * `addr`: An address that is returned from `ref_to_address()`
    fn address_to_ref(addr: Address) -> ObjectReference;

    /// Dump debugging information for an object.
    ///
    /// Arguments:
    /// * `object`: The object to be dumped.
    fn dump_object(object: ObjectReference);

    /// Return if an object is valid from the runtime point of view. This is used
    /// to debug MMTk.
    fn is_object_sane(_object: ObjectReference) -> bool {
        true
    }

    // --- Reference Glue ---

    /// The type of finalizable objects. This type is used when the binding registers and pops finalizable objects.
    /// For most languages, they can just use `ObjectReference` for the finalizable type, meaning that they are registering
    /// and popping a normal object reference as finalizable objects.
    type FinalizableType: Finalizable;

    // TODO: Should we also move the following methods about weak references to a trait (similar to the `Finalizable` trait)?

    /// Weak and soft references always clear the referent
    /// before enqueueing.
    ///
    /// Arguments:
    /// * `new_reference`: The reference whose referent is to be cleared.
    fn weakref_clear_referent(new_reference: ObjectReference) {
        Self::weakref_set_referent(new_reference, ObjectReference::NULL);
    }

    /// Get the referent from a weak reference object.
    ///
    /// Arguments:
    /// * `object`: The object reference.
    fn weakref_get_referent(object: ObjectReference) -> ObjectReference;

    /// Set the referent in a weak reference object.
    ///
    /// Arguments:
    /// * `reff`: The object reference for the reference.
    /// * `referent`: The referent object reference.
    fn weakref_set_referent(reff: ObjectReference, referent: ObjectReference);

    /// Check if the referent has been cleared.
    ///
    /// Arguments:
    /// * `referent`: The referent object reference.
    fn weakref_is_referent_cleared(referent: ObjectReference) -> bool {
        referent.is_null()
    }

    /// For weak reference types, if the referent is cleared during GC, the reference
    /// will be added to a queue, and MMTk will call this method to inform
    /// the VM about the changes for those references. This method is used
    /// to implement Java's ReferenceQueue.
    /// Note that this method is called for each type of weak references during GC, and
    /// the references slice will be cleared after this call is returned. That means
    /// MMTk will no longer keep these references alive once this method is returned.
    fn weakref_enqueue_references(references: &[ObjectReference], tls: VMWorkerThread);

    // --- Scanning ---

    /// Return true if the given object supports edge enqueuing.
    ///
    /// -   If this returns true, MMTk core will call `scan_object` on the object.
    /// -   Otherwise, MMTk core will call `scan_object_and_trace_edges` on the object.
    ///
    /// For maximum performance, the VM should support edge-enqueuing for as many objects as
    /// practical.  Also note that this method is called for every object to be scanned, so it
    /// must be fast.  The VM binding should avoid expensive checks and keep it as efficient as
    /// possible.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    fn support_edge_enqueuing(_tls: VMWorkerThread, _object: ObjectReference) -> bool {
        true
    }

    /// Delegated scanning of a object, visiting each reference field encountered.
    ///
    /// The VM shall call `edge_visitor.visit_edge` on each reference field.
    ///
    /// The VM may skip a reference field if it holds a null reference.  If the VM supports tagged
    /// references, it must skip tagged reference fields which are not holding references.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `edge_visitor`: Called back for each edge.
    fn scan_object<EV: EdgeVisitor<Self::VMEdge>>(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut EV,
    );

    /// Delegated scanning of a object, visiting each reference field encountered, and trace the
    /// objects pointed by each field.
    ///
    /// The VM shall call `object_tracer.trace_object` on the value held in each reference field,
    /// and assign the returned value back to the field.  If the VM uses tagged references, the
    /// value passed to `object_tracer.trace_object` shall be the `ObjectReference` to the object
    /// without any tag bits.
    ///
    /// The VM may skip a reference field if it holds a null reference.  If the VM supports tagged
    /// references, it must skip tagged reference fields which are not holding references.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `object_tracer`: Called back for the content of each edge.
    fn scan_object_and_trace_edges<OT: ObjectTracer>(
        _tls: VMWorkerThread,
        _object: ObjectReference,
        _object_tracer: &mut OT,
    ) {
        unreachable!("scan_object_and_trace_edges() will not be called when support_edge_enqueue() is always true.")
    }

    /// MMTk calls this method at the first time during a collection that thread's stacks
    /// have been scanned. This can be used (for example) to clean up
    /// obsolete compiled methods that are no longer being executed.
    ///
    /// Arguments:
    /// * `partial_scan`: Whether the scan was partial or full-heap.
    /// * `tls`: The GC thread that is performing the thread scan.
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: VMWorkerThread);

    /// Scan one mutator for stack roots.
    ///
    /// Some VM bindings may not be able to implement this method.
    /// For example, the VM binding may only be able to enumerate all threads and
    /// scan them while enumerating, but cannot scan stacks individually when given
    /// the references of threads.
    /// In that case, it can leave this method empty, and deal with stack
    /// roots in [`VMBinding::scan_vm_specific_roots`]. However, in that case, MMTk
    /// does not know those roots are stack roots, and cannot perform any possible
    /// optimization for the stack roots.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_roots_in_mutator_thread(
        tls: VMWorkerThread,
        mutator: &'static mut Mutator<Self>,
        factory: impl RootsWorkFactory<Self::VMEdge>,
    );

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_vm_specific_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory<Self::VMEdge>);

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;

    /// Prepare for another round of root scanning in the same GC. Some GC algorithms
    /// need multiple transitive closures, and each transitive closure starts from
    /// root scanning. We expect the binding to provide the same root set for every
    /// round of root scanning in the same GC. Bindings can use this call to get
    /// ready for another round of root scanning to make sure that the same root
    /// set will be returned in the upcoming calls of root scanning methods,
    /// such as [`crate::vm::VMBinding::scan_roots_in_mutator_thread`] and
    /// [`crate::vm::VMBinding::scan_vm_specific_roots`].
    fn prepare_for_roots_re_scanning();

    /// Process weak references.
    ///
    /// This function is called after a transitive closure is completed.
    ///
    /// MMTk core enables the VM binding to do the following in this function:
    ///
    /// 1.  Query if an object is already reached in this transitive closure.
    /// 2.  Get the new address of an object if it is already reached.
    /// 3.  Keep an object and its descendents alive if not yet reached.
    /// 4.  Request this function to be called again after transitive closure is finished again.
    ///
    /// The VM binding can query if an object is currently reached by calling
    /// `ObjectReference::is_reachable()`.
    ///
    /// If an object is already reached, the VM binding can get its new address by calling
    /// `ObjectReference::get_forwarded_object()` as the object may have been moved.
    ///
    /// If an object is not yet reached, the VM binding can keep that object and its descendents
    /// alive.  To do this, the VM binding should use `tracer_context.with_tracer` to get access to
    /// an `ObjectTracer`, and then call its `trace_object(object)` method.  The `trace_object`
    /// method will return the new address of the `object` if it moved the object, or its original
    /// address if not moved.  Implementation-wise, the `ObjectTracer` may contain an internal
    /// queue for newly traced objects, and will flush the queue when `tracer_context.with_tracer`
    /// returns. Therefore, it is recommended to reuse the `ObjectTracer` instance to trace
    /// multiple objects.
    ///
    /// *Note that if `trace_object` is called on an already reached object, the behavior will be
    /// equivalent to `ObjectReference::get_forwarded_object()`.  It will return the new address if
    /// the GC already moved the object when tracing that object, or the original address if the GC
    /// did not move the object when tracing it.  In theory, the VM binding can use `trace_object`
    /// wherever `ObjectReference::get_forwarded_object()` is needed.  However, if a VM never
    /// resurrects objects, it should completely avoid touching `tracer_context`, and exclusively
    /// use `ObjectReference::get_forwarded_object()` to get new addresses of objects.  By doing
    /// so, the VM binding can avoid accidentally resurrecting objects.*
    ///
    /// The VM binding can return `true` from `process_weak_refs` to request `process_weak_refs`
    /// to be called again after the MMTk core finishes transitive closure again from the objects
    /// newly visited by `ObjectTracer::trace_object`.  This is useful if a VM supports multiple
    /// levels of reachabilities (such as Java) or ephemerons.
    ///
    /// Implementation-wise, this function is called as the "sentinel" of the `VMRefClosure` work
    /// bucket, which means it is called when all work packets in that bucket have finished.  The
    /// `tracer_context` expands the transitive closure by adding more work packets in the same
    /// bucket.  This means if `process_weak_refs` returns true, those work packets will have
    /// finished (completing the transitive closure) by the time `process_weak_refs` is called
    /// again.  The VM binding can make use of this by adding custom work packets into the
    /// `VMRefClosure` bucket.  The bucket will be `VMRefForwarding`, instead, when forwarding.
    /// See below.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `tracer_context`: Use this to get access an `ObjectTracer` and use it to retain and
    ///   update weak references.
    ///
    /// This function shall return true if this function needs to be called again after the GC
    /// finishes expanding the transitive closure from the objects kept alive.
    fn process_weak_refs(
        _worker: &mut GCWorker<Self>,
        _tracer_context: impl ObjectTracerContext<Self>,
    ) -> bool {
        false
    }

    /// Forward weak references.
    ///
    /// This function will only be called in the forwarding stage when using the mark-compact GC
    /// algorithm.  Mark-compact computes transive closure twice during each GC.  It marks objects
    /// in the first transitive closure, and forward references in the second transitive closure.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `tracer_context`: Use this to get access an `ObjectTracer` and use it to update weak
    ///   references.
    fn forward_weak_refs(
        _worker: &mut GCWorker<Self>,
        _tracer_context: impl ObjectTracerContext<Self>,
    ) {
    }
}
