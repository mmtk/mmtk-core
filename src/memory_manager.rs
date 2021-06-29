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

use crate::mmtk::MMTK;
use crate::plan::AllocationSemantics;
use crate::plan::{Mutator, MutatorContext};
use crate::scheduler::GCWorker;
use crate::scheduler::Work;
use crate::scheduler::WorkBucketStage;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::constants::{LOG_BYTES_IN_PAGE, MIN_OBJECT_SIZE};
use crate::util::heap::layout::vm_layout_constants::HEAP_END;
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::vm::VMBinding;
use std::sync::atomic::Ordering;

/// Run the main loop for the GC controller thread. This method does not return.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that will be used as the GC controller.
pub fn start_control_collector<VM: VMBinding>(mmtk: &MMTK<VM>, tls: VMWorkerThread) {
    mmtk.plan.base().control_collector_context.run(tls);
}

/// Initialize an MMTk instance. A VM should call this method after creating an [MMTK](../mmtk/struct.MMTK.html)
/// instance but before using any of the methods provided in MMTk. This method will attempt to initialize a
/// logger. If the VM would like to use its own logger, it should initialize the logger before calling this method.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance to initialize.
/// * `heap_size`: The heap size for the MMTk instance in bytes.
pub fn gc_init<VM: VMBinding>(mmtk: &'static mut MMTK<VM>, heap_size: usize) {
    match crate::util::logger::try_init() {
        Ok(_) => debug!("MMTk initialized the logger."),
        Err(_) => debug!(
            "MMTk failed to initialize the logger. Possibly a logger has been initialized by user."
        ),
    }
    #[cfg(feature = "perf_counter")]
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
                    warn!("Current process has {} threads, process-wide perf event measurement will only include child threads spawned from this threadas", threads);
                }
            }
    }
    assert!(heap_size > 0, "Invalid heap size");
    mmtk.plan
        .gc_init(heap_size, &crate::VM_MAP, &mmtk.scheduler);
    info!("Initialized MMTk with {:?}", mmtk.options.plan);
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
    crate::plan::create_mutator(tls, mmtk)
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
pub fn post_alloc<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    refer: ObjectReference,
    bytes: usize,
    semantics: AllocationSemantics,
) {
    mutator.post_alloc(refer, bytes, semantics);
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

/// Run the main loop of a GC worker. This method does not return.
///
/// Arguments:
/// * `tls`: The thread that will be used as the GC worker.
/// * `worker`: A reference to the GC worker.
/// * `mmtk`: A reference to an MMTk instance.
pub fn start_worker<VM: VMBinding>(
    tls: VMWorkerThread,
    worker: &mut GCWorker<VM>,
    mmtk: &'static MMTK<VM>,
) {
    worker.init(tls);
    worker.set_local(mmtk.plan.create_worker_local(tls, mmtk));
    worker.run(mmtk);
}

/// Allow MMTk to trigger garbage collection. A VM should only call this method when it is ready for the mechanisms required for
/// collection during the boot process. MMTk will invoke Collection::spawn_worker_thread() to create GC threads during
/// this funciton call.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that wants to enable the collection. This value will be passed back to the VM in
///   Collection::spawn_worker_thread() so that the VM knows the context.
pub fn enable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>, tls: VMThread) {
    mmtk.scheduler.initialize(mmtk.options.threads, mmtk, tls);
    VM::VMCollection::spawn_worker_thread(tls, None); // spawn controller thread
    mmtk.plan.base().initialized.store(true, Ordering::SeqCst);
}

/// Process MMTk run-time options.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `name`: The name of the option.
/// * `value`: The value of the option (as a string).
pub fn process<VM: VMBinding>(mmtk: &'static MMTK<VM>, name: &str, value: &str) -> bool {
    // Note that currently we cannot process options for setting plan,
    // as we have set plan when creating an MMTK instance, and processing options is after creating on an instance.
    // The only way to set plan is to use the env var 'MMTK_PLAN'.
    // FIXME: We should remove this function, and ask for options when creating an MMTk instance.
    assert!(name != "plan");

    unsafe { mmtk.options.process(name, value) }
}

/// Return used memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn used_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_pages_used() << LOG_BYTES_IN_PAGE
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

/// Is the object in the mapped memory? The runtime can use this function to check
/// if an object is in MMTk heap.
///
/// Arguments:
/// * `object`: The object reference to query.
pub fn is_mapped_object(object: ObjectReference) -> bool {
    object.is_mapped()
}

/// Is the address in the mapped memory? The runtime can use this function to check
/// if an address is mapped by MMTk. Note that this is different than is_mapped_object().
/// For malloc spaces, MMTk does not map those addresses (malloc does the mmap), so
/// this function will return false, but is_mapped_object will return true if the address
/// is actually a valid object in malloc spaces. To check if an object is in our heap,
/// the runtime should always use is_mapped_object(). This function is_mapped_address()
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

/// Add a reference to the list of weak references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The weak reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_weak_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_weak_candidate::<VM>(reff, referent);
}

/// Add a reference to the list of soft references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The soft reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_soft_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_soft_candidate::<VM>(reff, referent);
}

/// Add a reference to the list of phantom references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The phantom reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_phantom_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_phantom_candidate::<VM>(reff, referent);
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
pub fn add_finalizer<VM: VMBinding>(mmtk: &'static MMTK<VM>, object: ObjectReference) {
    if mmtk.options.no_finalizer {
        warn!("add_finalizer() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor.lock().unwrap().add(object);
}

/// Get an object that is ready for finalization. After each GC, if any registered object is not
/// alive, this call will return one of the objects. MMTk will retain the liveness of those objects
/// until they are popped through this call. Once an object is popped, it is the responsibility of
/// the VM to make sure they are properly finalized before reclaimed by the GC. This call is non-blocking,
/// and will return None if no object is ready for finalization.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn get_finalized_object<VM: VMBinding>(mmtk: &'static MMTK<VM>) -> Option<ObjectReference> {
    if mmtk.options.no_finalizer {
        warn!("get_object_for_finalization() is called when no_finalizer = true");
    }

    mmtk.finalizable_processor
        .lock()
        .unwrap()
        .get_ready_object()
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
pub fn add_work_packet<VM: VMBinding, W: Work<MMTK<VM>>>(
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
    packets: Vec<Box<dyn Work<MMTK<VM>>>>,
) {
    mmtk.scheduler.work_buckets[bucket].bulk_add(packets)
}
