//! VM-to-MMTk interface: safe Rust APIs.
//!
//! This module provides a safe Rust API for mmtk-core.
//! We expect the VM binding to inherit and extend this API by:
//! 1. adding their VM-specific functions
//! 2. exposing the functions to native if necessary. And the VM binding needs to manage the unsafety
//!    for exposing this safe API to FFI.
//!
//! For example, for mutators, this API provides a `Box<Mutator>`, and requires a `&mut Mutator` for allocation.
//! A VM binding can borrow a mutable reference directly from `Box<Mutator>`, and call `alloc()`. Alternatively,
//! it can turn the `Box` pointer to a native pointer (`*mut Mutator`), and forge a mut reference from the native
//! pointer. Either way, the VM binding code needs to guarantee the safety.

use crate::mmtk::MMTKBuilder;
use crate::mmtk::MMTK;
use crate::plan::AllocationSemantics;
use crate::plan::{Mutator, MutatorContext};
use crate::scheduler::WorkBucketStage;
use crate::scheduler::{GCController, GCWork, GCWorker};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::constants::{LOG_BYTES_IN_PAGE, MIN_OBJECT_SIZE};
use crate::util::heap::layout::vm_layout_constants::HEAP_END;
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::edge_shape::MemorySlice;
use crate::vm::ReferenceGlue;
use crate::vm::VMBinding;
use std::sync::atomic::Ordering;

/// Initialize an MMTk instance. A VM should call this method after creating an [MMTK](../mmtk/struct.MMTK.html)
/// instance but before using any of the methods provided in MMTk (except `process()` and `process_bulk()`).
///
/// We expect a binding to ininitialize MMTk in the following steps:
///
/// 1. Create an [MMTKBuilder](../mmtk/struct.MMTKBuilder.html) instance.
/// 2. Set command line options for MMTKBuilder by [process()](./fn.process.html) or [process_bulk()](./fn.process_bulk.html).
/// 3. Initialize MMTk by calling this function, `mmtk_init()`, and pass the builder earlier. This call will return an MMTK instance.
///    Usually a binding store the MMTK instance statically as a singleton. We plan to allow multiple instances, but this is not yet fully
///    supported. Currently we assume a binding will only need one MMTk instance.
/// 4. Enable garbage collection in MMTk by [enable_collection()](./fn.enable_collection.html). A binding should only call this once its
///    thread system is ready. MMTk will not trigger garbage collection before this call.
///
/// Note that this method will attempt to initialize a logger. If the VM would like to use its own logger, it should initialize the logger before calling this method.
/// Note that, to allow MMTk to do GC properly, `initialize_collection()` needs to be called after this call when
/// the VM's thread system is ready to spawn GC workers.
///
/// Note that this method returns a boxed pointer of MMTK, which means MMTk has a bound lifetime with the box pointer. However, some of our current APIs assume
/// that MMTk has a static lifetime, which presents a mismatch with this API. We plan to address the lifetime issue in the future. At this point, we recommend a binding
/// to 'expand' the lifetime for the boxed pointer to static. There could be multiple ways to achieve it: 1. `Box::leak()` will turn the box pointer to raw pointer
/// which has static lifetime, 2. create MMTK as a lazily initialized static variable
/// (see [what we do for our dummy binding](https://github.com/mmtk/mmtk-core/blob/master/vmbindings/dummyvm/src/lib.rs#L42))
///
/// Arguments:
/// * `builder`: The reference to a MMTk builder.
pub fn mmtk_init<VM: VMBinding>(builder: &MMTKBuilder) -> Box<MMTK<VM>> {
    match crate::util::logger::try_init() {
        Ok(_) => debug!("MMTk initialized the logger."),
        Err(_) => debug!(
            "MMTk failed to initialize the logger. Possibly a logger has been initialized by user."
        ),
    }
    #[cfg(all(feature = "perf_counter", target_os = "linux"))]
    {
        use std::fs::File;
        use std::io::Read;
        let mut status = File::open("/proc/self/status").unwrap();
        let mut contents = String::new();
        status.read_to_string(&mut contents).unwrap();
        for line in contents.lines() {
            let split: Vec<&str> = line.split('\t').collect();
            if split[0] == "Threads:" {
                let threads = split[1].parse::<i32>().unwrap();
                if threads != 1 {
                    warn!("Current process has {} threads, process-wide perf event measurement will only include child threads spawned from this thread", threads);
                }
            }
        }
    }
    let mmtk = builder.build();
    info!("Initialized MMTk with {:?}", *mmtk.options.plan);
    #[cfg(feature = "extreme_assertions")]
    warn!("The feature 'extreme_assertions' is enabled. MMTk will run expensive run-time checks. Slow performance should be expected.");
    Box::new(mmtk)
}

/// Request MMTk to create a mutator for the given thread. For performance reasons, A VM should
/// store the returned mutator in a thread local storage that can be accessed efficiently.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that will be associated with the mutator.
pub fn bind_mutator<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    tls: VMMutatorThread,
) -> Box<Mutator<VM>> {
    let mutator = crate::plan::create_mutator(tls, mmtk);

    const LOG_ALLOCATOR_MAPPING: bool = false;
    if LOG_ALLOCATOR_MAPPING {
        info!("{:?}", mutator.config);
    }
    mutator
}

/// Reclaim a mutator that is no longer needed.
///
/// Arguments:
/// * `mutator`: A reference to the mutator to be destroyed.
pub fn destroy_mutator<VM: VMBinding>(mutator: Box<Mutator<VM>>) {
    drop(mutator);
}

/// Flush the mutator's local states.
///
/// Arguments:
/// * `mutator`: A reference to the mutator.
pub fn flush_mutator<VM: VMBinding>(mutator: &mut Mutator<VM>) {
    mutator.flush()
}

/// Allocate memory for an object. For performance reasons, a VM should
/// implement the allocation fast-path on their side rather than just calling this function.
///
/// Arguments:
/// * `mutator`: The mutator to perform this allocation request.
/// * `size`: The number of bytes required for the object.
/// * `align`: Required alignment for the object.
/// * `offset`: Offset associated with the alignment.
/// * `semantics`: The allocation semantic required for the allocation.
#[inline(always)]
pub fn alloc<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    size: usize,
    align: usize,
    offset: isize,
    semantics: AllocationSemantics,
) -> Address {
    // MMTk has assumptions about minimal object size.
    // We need to make sure that all allocations comply with the min object size.
    // Ideally, we check the allocation size, and if it is smaller, we transparently allocate the min
    // object size (the VM does not need to know this). However, for the VM bindings we support at the moment,
    // their object sizes are all larger than MMTk's min object size, so we simply put an assertion here.
    // If you plan to use MMTk with a VM with its object size smaller than MMTk's min object size, you should
    // meet the min object size in the fastpath.
    debug_assert!(size >= MIN_OBJECT_SIZE);
    mutator.alloc(size, align, offset, semantics)
}

/// Perform post-allocation actions, usually initializing object metadata. For many allocators none are
/// required. For performance reasons, a VM should implement the post alloc fast-path on their side
/// rather than just calling this function.
///
/// Arguments:
/// * `mutator`: The mutator to perform post-alloc actions.
/// * `refer`: The newly allocated object.
/// * `bytes`: The size of the space allocated for the object (in bytes).
/// * `semantics`: The allocation semantics used for the allocation.
#[inline(always)]
pub fn post_alloc<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    refer: ObjectReference,
    bytes: usize,
    semantics: AllocationSemantics,
) {
    mutator.post_alloc(refer, bytes, semantics);
}

/// The *subsuming* write barrier by MMTk. For performance reasons, a VM should implement the write barrier
/// fast-path on their side rather than just calling this function.
///
/// For a correct barrier implementation, a VM binding needs to choose one of the following options:
/// * Use subsuming barrier `object_reference_write`
/// * Use both `object_reference_write_pre` and `object_reference_write_post`, or both, if the binding has difficulty delegating the store to mmtk-core with the subsuming barrier.
/// * Implement fast-path on the VM side, and call the generic api `object_reference_slow` as barrier slow-path call.
/// * Implement fast-path on the VM side, and do a specialized slow-path call.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: The modified source object.
/// * `slot`: The location of the field to be modified.
/// * `target`: The target for the write operation.
#[inline(always)]
pub fn object_reference_write<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    src: ObjectReference,
    slot: VM::VMEdge,
    target: ObjectReference,
) {
    mutator.barrier().object_reference_write(src, slot, target);
}

/// The write barrier by MMTk. This is a *pre* write barrier, which we expect a binding to call
/// *before* it modifies an object. For performance reasons, a VM should implement the write barrier
/// fast-path on their side rather than just calling this function.
///
/// For a correct barrier implementation, a VM binding needs to choose one of the following options:
/// * Use subsuming barrier `object_reference_write`
/// * Use both `object_reference_write_pre` and `object_reference_write_post`, or both, if the binding has difficulty delegating the store to mmtk-core with the subsuming barrier.
/// * Implement fast-path on the VM side, and call the generic api `object_reference_slow` as barrier slow-path call.
/// * Implement fast-path on the VM side, and do a specialized slow-path call.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: The modified source object.
/// * `slot`: The location of the field to be modified.
/// * `target`: The target for the write operation.
#[inline(always)]
pub fn object_reference_write_pre<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    src: ObjectReference,
    slot: VM::VMEdge,
    target: ObjectReference,
) {
    mutator
        .barrier()
        .object_reference_write_pre(src, slot, target);
}

/// The write barrier by MMTk. This is a *post* write barrier, which we expect a binding to call
/// *after* it modifies an object. For performance reasons, a VM should implement the write barrier
/// fast-path on their side rather than just calling this function.
///
/// For a correct barrier implementation, a VM binding needs to choose one of the following options:
/// * Use subsuming barrier `object_reference_write`
/// * Use both `object_reference_write_pre` and `object_reference_write_post`, or both, if the binding has difficulty delegating the store to mmtk-core with the subsuming barrier.
/// * Implement fast-path on the VM side, and call the generic api `object_reference_slow` as barrier slow-path call.
/// * Implement fast-path on the VM side, and do a specialized slow-path call.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: The modified source object.
/// * `slot`: The location of the field to be modified.
/// * `target`: The target for the write operation.
#[inline(always)]
pub fn object_reference_write_post<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    src: ObjectReference,
    slot: VM::VMEdge,
    target: ObjectReference,
) {
    mutator
        .barrier()
        .object_reference_write_post(src, slot, target);
}

/// The *subsuming* memory region copy barrier by MMTk.
/// This is called when the VM tries to copy a piece of heap memory to another.
/// The data within the slice does not necessarily to be all valid pointers,
/// but the VM binding will be able to filter out non-reference values on edge iteration.
///
/// For VMs that performs a heap memory copy operation, for example OpenJDK's array copy operation, the binding needs to
/// call `memory_region_copy*` APIs. Same as `object_reference_write*`, the binding can choose either the subsuming barrier,
/// or the pre/post barrier.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: Source memory slice to copy from.
/// * `dst`: Destination memory slice to copy to.
///
/// The size of `src` and `dst` shoule be equal
#[inline(always)]
pub fn memory_region_copy<VM: VMBinding>(
    mutator: &'static mut Mutator<VM>,
    src: VM::VMMemorySlice,
    dst: VM::VMMemorySlice,
) {
    debug_assert_eq!(src.bytes(), dst.bytes());
    mutator.barrier().memory_region_copy(src, dst);
}

/// The *generic* memory region copy *pre* barrier by MMTk, which we expect a binding to call
/// *before* it performs memory copy.
/// This is called when the VM tries to copy a piece of heap memory to another.
/// The data within the slice does not necessarily to be all valid pointers,
/// but the VM binding will be able to filter out non-reference values on edge iteration.
///
/// For VMs that performs a heap memory copy operation, for example OpenJDK's array copy operation, the binding needs to
/// call `memory_region_copy*` APIs. Same as `object_reference_write*`, the binding can choose either the subsuming barrier,
/// or the pre/post barrier.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: Source memory slice to copy from.
/// * `dst`: Destination memory slice to copy to.
///
/// The size of `src` and `dst` shoule be equal
#[inline(always)]
pub fn memory_region_copy_pre<VM: VMBinding>(
    mutator: &'static mut Mutator<VM>,
    src: VM::VMMemorySlice,
    dst: VM::VMMemorySlice,
) {
    debug_assert_eq!(src.bytes(), dst.bytes());
    mutator.barrier().memory_region_copy_pre(src, dst);
}

/// The *generic* memory region copy *post* barrier by MMTk, which we expect a binding to call
/// *after* it performs memory copy.
/// This is called when the VM tries to copy a piece of heap memory to another.
/// The data within the slice does not necessarily to be all valid pointers,
/// but the VM binding will be able to filter out non-reference values on edge iteration.
///
/// For VMs that performs a heap memory copy operation, for example OpenJDK's array copy operation, the binding needs to
/// call `memory_region_copy*` APIs. Same as `object_reference_write*`, the binding can choose either the subsuming barrier,
/// or the pre/post barrier.
///
/// Arguments:
/// * `mutator`: The mutator for the current thread.
/// * `src`: Source memory slice to copy from.
/// * `dst`: Destination memory slice to copy to.
///
/// The size of `src` and `dst` shoule be equal
#[inline(always)]
pub fn memory_region_copy_post<VM: VMBinding>(
    mutator: &'static mut Mutator<VM>,
    src: VM::VMMemorySlice,
    dst: VM::VMMemorySlice,
) {
    debug_assert_eq!(src.bytes(), dst.bytes());
    mutator.barrier().memory_region_copy_post(src, dst);
}

/// Return an AllocatorSelector for the given allocation semantic. This method is provided
/// so that VM compilers may call it to help generate allocation fast-path.
///
/// Arguments:
/// * `mmtk`: The reference to an MMTk instance.
/// * `semantics`: The allocation semantic to query.
pub fn get_allocator_mapping<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    semantics: AllocationSemantics,
) -> AllocatorSelector {
    mmtk.plan.get_allocator_mapping()[semantics]
}

/// The standard malloc. MMTk either uses its own allocator, or forward the call to a
/// library malloc.
pub fn malloc(size: usize) -> Address {
    crate::util::malloc::malloc(size)
}

/// The standard malloc except that with the feature `malloc_counted_size`, MMTk will count the allocated memory into its heap size.
/// Thus the method requires a reference to an MMTk instance. MMTk either uses its own allocator, or forward the call to a
/// library malloc.
#[cfg(feature = "malloc_counted_size")]
pub fn counted_malloc<VM: VMBinding>(mmtk: &MMTK<VM>, size: usize) -> Address {
    crate::util::malloc::counted_malloc(mmtk, size)
}

/// The standard calloc.
pub fn calloc(num: usize, size: usize) -> Address {
    crate::util::malloc::calloc(num, size)
}

/// The standard calloc except that with the feature `malloc_counted_size`, MMTk will count the allocated memory into its heap size.
/// Thus the method requires a reference to an MMTk instance.
#[cfg(feature = "malloc_counted_size")]
pub fn counted_calloc<VM: VMBinding>(mmtk: &MMTK<VM>, num: usize, size: usize) -> Address {
    crate::util::malloc::counted_calloc(mmtk, num, size)
}

/// The standard realloc.
pub fn realloc(addr: Address, size: usize) -> Address {
    crate::util::malloc::realloc(addr, size)
}

/// The standard realloc except that with the feature `malloc_counted_size`, MMTk will count the allocated memory into its heap size.
/// Thus the method requires a reference to an MMTk instance, and the size of the existing memory that will be reallocated.
/// The `addr` in the arguments must be an address that is earlier returned from MMTk's `malloc()`, `calloc()` or `realloc()`.
#[cfg(feature = "malloc_counted_size")]
pub fn realloc_with_old_size<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    addr: Address,
    size: usize,
    old_size: usize,
) -> Address {
    crate::util::malloc::realloc_with_old_size(mmtk, addr, size, old_size)
}

/// The standard free.
/// The `addr` in the arguments must be an address that is earlier returned from MMTk's `malloc()`, `calloc()` or `realloc()`.
pub fn free(addr: Address) {
    crate::util::malloc::free(addr)
}

/// The standard free except that with the feature `malloc_counted_size`, MMTk will count the allocated memory into its heap size.
/// Thus the method requires a reference to an MMTk instance, and the size of the memory to free.
/// The `addr` in the arguments must be an address that is earlier returned from MMTk's `malloc()`, `calloc()` or `realloc()`.
#[cfg(feature = "malloc_counted_size")]
pub fn free_with_size<VM: VMBinding>(mmtk: &MMTK<VM>, addr: Address, old_size: usize) {
    crate::util::malloc::free_with_size(mmtk, addr, old_size)
}

/// Poll for GC. MMTk will decide if a GC is needed. If so, this call will block
/// the current thread, and trigger a GC. Otherwise, it will simply return.
/// Usually a binding does not need to call this function. MMTk will poll for GC during its allocation.
/// However, if a binding uses counted malloc (which won't poll for GC), they may want to poll for GC manually.
/// This function should only be used by mutator threads.
pub fn gc_poll<VM: VMBinding>(mmtk: &MMTK<VM>, tls: VMMutatorThread) {
    use crate::vm::{ActivePlan, Collection};
    debug_assert!(
        VM::VMActivePlan::is_mutator(tls.0),
        "gc_poll() can only be called by a mutator thread."
    );

    let plan = mmtk.get_plan();
    if plan.should_trigger_gc_when_heap_is_full() && plan.poll(false, None) {
        debug!("Collection required");
        assert!(plan.is_initialized(), "GC is not allowed here: collection is not initialized (did you call initialize_collection()?).");
        VM::VMCollection::block_for_gc(tls);
    }
}

/// Run the main loop for the GC controller thread. This method does not return.
///
/// Arguments:
/// * `tls`: The thread that will be used as the GC controller.
/// * `gc_controller`: The execution context of the GC controller threa.
///   It is the `GCController` passed to `Collection::spawn_gc_thread`.
/// * `mmtk`: A reference to an MMTk instance.
pub fn start_control_collector<VM: VMBinding>(
    _mmtk: &'static MMTK<VM>,
    tls: VMWorkerThread,
    gc_controller: &mut GCController<VM>,
) {
    gc_controller.run(tls);
}

/// Run the main loop of a GC worker. This method does not return.
///
/// Arguments:
/// * `tls`: The thread that will be used as the GC worker.
/// * `worker`: The execution context of the GC worker thread.
///   It is the `GCWorker` passed to `Collection::spawn_gc_thread`.
/// * `mmtk`: A reference to an MMTk instance.
pub fn start_worker<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    tls: VMWorkerThread,
    worker: &mut GCWorker<VM>,
) {
    worker.run(tls, mmtk);
}

/// Initialize the scheduler and GC workers that are required for doing garbage collections.
/// This is a mandatory call for a VM during its boot process once its thread system
/// is ready. This should only be called once. This call will invoke Collection::spawn_gc_thread()
/// to create GC threads.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that wants to enable the collection. This value will be passed back to the VM in
///   Collection::spawn_gc_thread() so that the VM knows the context.
pub fn initialize_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>, tls: VMThread) {
    assert!(
        !mmtk.plan.is_initialized(),
        "MMTk collection has been initialized (was initialize_collection() already called before?)"
    );
    mmtk.scheduler.spawn_gc_threads(mmtk, tls);
    mmtk.plan.base().initialized.store(true, Ordering::SeqCst);
}

/// Allow MMTk to trigger garbage collection when heap is full. This should only be used in pair with disable_collection().
/// See the comments on disable_collection(). If disable_collection() is not used, there is no need to call this function at all.
/// Note this call is not thread safe, only one VM thread should call this.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn enable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>) {
    debug_assert!(
        !mmtk.plan.should_trigger_gc_when_heap_is_full(),
        "enable_collection() is called when GC is already enabled."
    );
    mmtk.plan
        .base()
        .trigger_gc_when_heap_is_full
        .store(true, Ordering::SeqCst);
}

/// Disallow MMTk to trigger garbage collection. When collection is disabled, you can still allocate through MMTk. But MMTk will
/// not trigger a GC even if the heap is full. In such a case, the allocation will exceed the MMTk's heap size (the soft heap limit).
/// However, there is no guarantee that the physical allocation will succeed, and if it succeeds, there is no guarantee that further allocation
/// will keep succeeding. So if a VM disables collection, it needs to allocate with careful consideration to make sure that the physical memory
/// allows the amount of allocation. We highly recommend not using this method. However, we support this to accomodate some VMs that require this
/// behavior. This call does not disable explicit GCs (through handle_user_collection_request()).
/// Note this call is not thread safe, only one VM thread should call this.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn disable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>) {
    debug_assert!(
        mmtk.plan.should_trigger_gc_when_heap_is_full(),
        "disable_collection() is called when GC is not enabled."
    );
    mmtk.plan
        .base()
        .trigger_gc_when_heap_is_full
        .store(false, Ordering::SeqCst);
}

/// Process MMTk run-time options. Returns true if the option is processed successfully.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `name`: The name of the option.
/// * `value`: The value of the option (as a string).
pub fn process(builder: &mut MMTKBuilder, name: &str, value: &str) -> bool {
    builder.set_option(name, value)
}

/// Process multiple MMTk run-time options. Returns true if all the options are processed successfully.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `options`: a string that is key value pairs separated by white spaces, e.g. "threads=1 stress_factor=4096"
pub fn process_bulk(builder: &mut MMTKBuilder, options: &str) -> bool {
    builder.set_options_bulk_by_str(options)
}

/// Return used memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn used_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_used_pages() << LOG_BYTES_IN_PAGE
}

/// Return free memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn free_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_free_pages() << LOG_BYTES_IN_PAGE
}

/// Return the starting address of the heap. *Note that currently MMTk uses
/// a fixed address range as heap.*
pub fn starting_heap_address() -> Address {
    HEAP_START
}

/// Return the ending address of the heap. *Note that currently MMTk uses
/// a fixed address range as heap.*
pub fn last_heap_address() -> Address {
    HEAP_END
}

/// Return the total memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn total_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

/// Trigger a garbage collection as requested by the user.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that triggers this collection request.
pub fn handle_user_collection_request<VM: VMBinding>(mmtk: &MMTK<VM>, tls: VMMutatorThread) {
    mmtk.plan.handle_user_collection_request(tls, false);
}

/// Is the object alive?
///
/// Arguments:
/// * `object`: The object reference to query.
pub fn is_live_object(object: ObjectReference) -> bool {
    object.is_live()
}

/// Check if `addr` is the address of an object reference to an MMTk object.
///
/// Concretely:
/// 1.  Return true if `addr.to_object_reference()` is a valid object reference to an object in any
///     space in MMTk.
/// 2.  Also return true if there exists an `objref: ObjectReference` such that
///     -   `objref` is a valid object reference to an object in any space in MMTk, and
///     -   `lo <= objref.to_address() < hi`, where
///         -   `lo = addr.align_down(ALLOC_BIT_REGION_SIZE)` and
///         -   `hi = lo + ALLOC_BIT_REGION_SIZE` and
///         -   `ALLOC_BIT_REGION_SIZE` is [`crate::util::is_mmtk_object::ALLOC_BIT_REGION_SIZE`].
///             It is the byte granularity of the alloc bit.
/// 3.  Return false otherwise.  This function never panics.
///
/// Case 2 means **this function is imprecise for misaligned addresses**.
/// This function uses the "alloc bits" side metadata, i.e. a bitmap.
/// For space efficiency, each bit of the bitmap governs a small region of memory.
/// The size of a region is currently defined as the [minimum object size](crate::util::constants::MIN_OBJECT_SIZE),
/// which is currently defined as the [word size](crate::util::constants::BYTES_IN_WORD),
/// which is 4 bytes on 32-bit systems or 8 bytes on 64-bit systems.
/// The alignment of a region is also the region size.
/// If an alloc bit is `1`, the bitmap cannot tell which address within the 4-byte or 8-byte region
/// is the valid object reference.
/// Therefore, if the input `addr` is not properly aligned, but is close to a valid object
/// reference, this function may still return true.
///
/// For the reason above, the VM **must check if `addr` is properly aligned** before calling this
/// function.  For most VMs, valid object references are always aligned to the word size, so
/// checking `addr.is_aligned_to(BYTES_IN_WORD)` should usually work.  If you are paranoid, you can
/// always check against [`crate::util::is_mmtk_object::ALLOC_BIT_REGION_SIZE`].
///
/// This function is useful for conservative root scanning.  The VM can iterate through all words in
/// a stack, filter out zeros, misaligned words, obviously out-of-range words (such as addresses
/// greater than `0x0000_7fff_ffff_ffff` on Linux on x86_64), and use this function to deside if the
/// word is really a reference.
///
/// Note: This function has special behaviors if the VM space (enabled by the `vm_space` feature)
/// is present.  See `crate::plan::global::BasePlan::vm_space`.
///
/// Argument:
/// * `addr`: An arbitrary address.
#[cfg(feature = "is_mmtk_object")]
pub fn is_mmtk_object(addr: Address) -> bool {
    use crate::mmtk::SFT_MAP;
    use crate::policy::sft_map::SFTMap;
    SFT_MAP.get_checked(addr).is_mmtk_object(addr)
}

/// Return true if the `object` lies in a region of memory where
/// -   only MMTk can allocate into, or
/// -   only MMTk's delegated memory allocator (such as a malloc implementation) can allocate into
///     for allocation requests from MMTk.
/// Return false otherwise.  This function never panics.
///
/// Particularly, if this function returns true, `object` cannot be an object allocated by the VM
/// itself.
///
/// If this function returns true, the object cannot be allocate by the `malloc` function called by
/// the VM, either. In other words, if the `MallocSpace` of MMTk called `malloc` to allocate the
/// object for the VM in response to `memory_manager::alloc`, this function will return true; but
/// if the VM directly called `malloc` to allocate the object, this function will return false.
///
/// If `is_mmtk_object(object.to_address())` returns true, `is_in_mmtk_spaces(object)` must also
/// return true.
///
/// This function is useful if an object reference in the VM can be either a pointer into the MMTk
/// heap, or a pointer to non-MMTk objects.  If the VM has a pre-built boot image that contains
/// primordial objects, or if the VM has its own allocator or uses any third-party allocators, or
/// if the VM allows an object reference to point to native objects such as C++ objects, this
/// function can distinguish between MMTk-allocated objects and other objects.
///
/// Note: This function has special behaviors if the VM space (enabled by the `vm_space` feature)
/// is present.  See `crate::plan::global::BasePlan::vm_space`.
///
/// Arguments:
/// * `object`: The object reference to query.
pub fn is_in_mmtk_spaces(object: ObjectReference) -> bool {
    use crate::mmtk::SFT_MAP;
    use crate::policy::sft_map::SFTMap;
    SFT_MAP.get_checked(object.to_address()).is_in_space(object)
}

/// Is the address in the mapped memory? The runtime can use this function to check
/// if an address is mapped by MMTk. Note that this is different than is_in_mmtk_spaces().
/// For malloc spaces, MMTk does not map those addresses (malloc does the mmap), so
/// this function will return false, but is_in_mmtk_spaces will return true if the address
/// is actually a valid object in malloc spaces. To check if an object is in our heap,
/// the runtime should always use is_in_mmtk_spaces(). This function is_mapped_address()
/// may get removed at some point.
///
/// Arguments:
/// * `address`: The address to query.
// TODO: Do we really need this function? Can a runtime always use is_mapped_object()?
pub fn is_mapped_address(address: Address) -> bool {
    address.is_mapped()
}

/// Check that if a garbage collection is in progress and if the given
/// object is not movable.  If it is movable error messages are
/// logged and the system exits.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `object`: The object to check.
pub fn modify_check<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) {
    mmtk.plan.modify_check(object);
}

/// Add a reference to the list of weak references. A binding may
/// call this either when a weak reference is created, or when a weak reference is traced during GC.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The weak reference to add.
pub fn add_weak_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: ObjectReference) {
    mmtk.reference_processors.add_weak_candidate::<VM>(reff);
}

/// Add a reference to the list of soft references. A binding may
/// call this either when a weak reference is created, or when a weak reference is traced during GC.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The soft reference to add.
pub fn add_soft_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: ObjectReference) {
    mmtk.reference_processors.add_soft_candidate::<VM>(reff);
}

/// Add a reference to the list of phantom references. A binding may
/// call this either when a weak reference is created, or when a weak reference is traced during GC.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The phantom reference to add.
pub fn add_phantom_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: ObjectReference) {
    mmtk.reference_processors.add_phantom_candidate::<VM>(reff);
}

/// Generic hook to allow benchmarks to be harnessed. We do a full heap
/// GC, and then start recording statistics for MMTk.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that calls the function (and triggers a collection).
pub fn harness_begin<VM: VMBinding>(mmtk: &MMTK<VM>, tls: VMMutatorThread) {
    mmtk.harness_begin(tls);
}

/// Generic hook to allow benchmarks to be harnessed. We stop collecting
/// statistics, and print stats values.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn harness_end<VM: VMBinding>(mmtk: &'static MMTK<VM>) {
    mmtk.harness_end();
}

/// Register a finalizable object. MMTk will retain the liveness of
/// the object even if it is not reachable from the program.
/// Note that finalization upon exit is not supported.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance
/// * `object`: The object that has a finalizer
pub fn add_finalizer<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    object: <VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType,
) {
    if *mmtk.options.no_finalizer {
        warn!("add_finalizer() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor.lock().unwrap().add(object);
}

/// Pin an object. MMTk will make sure that the object does not move
/// during GC. Note that action cannot happen in some plans, eg, semispace.
/// It returns true if the pinning operation has been performed, i.e.,
/// the object status changed from non-pinned to pinned
///
/// Arguments:
/// * `object`: The object to be pinned
pub fn pin_object(object: ObjectReference) -> bool {
    use crate::mmtk::SFT_MAP;
    use crate::policy::sft_map::SFTMap;
    SFT_MAP.get_checked(object.to_address()).pin_object(object)
}

/// Unpin an object.
/// Returns true if the unpinning operation has been performed, i.e.,
/// the object status changed from pinned to non-pinned
///
/// Arguments:
/// * `object`: The object to be pinned
pub fn unpin_object(object: ObjectReference) -> bool {
    use crate::mmtk::SFT_MAP;
    use crate::policy::sft_map::SFTMap;
    SFT_MAP
        .get_checked(object.to_address())
        .unpin_object(object)
}

/// Check whether an object is currently pinned
///
/// Arguments:
/// * `object`: The object to be checked
pub fn is_pinned(object: ObjectReference) -> bool {
    use crate::mmtk::SFT_MAP;
    use crate::policy::sft_map::SFTMap;
    SFT_MAP
        .get_checked(object.to_address())
        .is_object_pinned(object)
}

/// Get an object that is ready for finalization. After each GC, if any registered object is not
/// alive, this call will return one of the objects. MMTk will retain the liveness of those objects
/// until they are popped through this call. Once an object is popped, it is the responsibility of
/// the VM to make sure they are properly finalized before reclaimed by the GC. This call is non-blocking,
/// and will return None if no object is ready for finalization.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn get_finalized_object<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
) -> Option<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType> {
    if *mmtk.options.no_finalizer {
        warn!("get_finalized_object() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor
        .lock()
        .unwrap()
        .get_ready_object()
}

/// Pop all the finalizers that were registered for finalization. The returned objects may or may not be ready for
/// finalization. After this call, MMTk's finalizer processor should have no registered finalizer any more.
///
/// This is useful for some VMs which require all finalizable objects to be finalized on exit.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn get_all_finalizers<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
) -> Vec<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType> {
    if *mmtk.options.no_finalizer {
        warn!("get_all_finalizers() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor
        .lock()
        .unwrap()
        .get_all_finalizers()
}

/// Pop finalizers that were registered and associated with a certain object. The returned objects may or may not be ready for finalization.
/// This is useful for some VMs that may manually execute finalize method for an object.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `object`: the given object that MMTk will pop its finalizers
pub fn get_finalizers_for<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    object: ObjectReference,
) -> Vec<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType> {
    if *mmtk.options.no_finalizer {
        warn!("get_finalizers() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor
        .lock()
        .unwrap()
        .get_finalizers_for(object)
}

/// Get the number of workers. MMTk spawns worker threads for the 'threads' defined in the options.
/// So the number of workers is derived from the threads option. Note the feature single_worker overwrites
/// the threads option, and force one worker thread.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn num_of_workers<VM: VMBinding>(mmtk: &'static MMTK<VM>) -> usize {
    mmtk.scheduler.num_workers()
}

/// Add a work packet to the given work bucket. Note that this simply adds the work packet to the given
/// work bucket, and the scheduler will decide when to execute the work packet.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `bucket`: Which work bucket to add this packet to.
/// * `packet`: The work packet to be added.
pub fn add_work_packet<VM: VMBinding, W: GCWork<VM>>(
    mmtk: &'static MMTK<VM>,
    bucket: WorkBucketStage,
    packet: W,
) {
    mmtk.scheduler.work_buckets[bucket].add(packet)
}

/// Bulk add a number of work packets to the given work bucket. Note that this simply adds the work packets
/// to the given work bucket, and the scheduler will decide when to execute the work packets.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `bucket`: Which work bucket to add these packets to.
/// * `packet`: The work packets to be added.
pub fn add_work_packets<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    bucket: WorkBucketStage,
    packets: Vec<Box<dyn GCWork<VM>>>,
) {
    mmtk.scheduler.work_buckets[bucket].bulk_add(packets)
}

/// Add a callback to be notified after the transitive closure is finished.
/// The callback should return true if it add more work packets to the closure bucket.
pub fn on_closure_end<VM: VMBinding>(mmtk: &'static MMTK<VM>, f: Box<dyn Send + Fn() -> bool>) {
    mmtk.scheduler.on_closure_end(f)
}
