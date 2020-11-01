//! VM-to-MMTk interface: safe Rust APIs.

/// This module provides a safe Rust API for mmtk-core.
/// We expect the VM binding to inherit and extend this API by:
/// 1. adding their VM-specific functions
/// 2. exposing the functions to native if necessary. And the VM binding needs to manage the unsafety
///    for exposing this safe API to FFI.

/// For example, for mutators, this API provides a Box<Mutator>, and requires a &mut Mutator for allocation.
/// A VM binding can borrow a mutable reference directly from Box<Mutator>, and call alloc(). Alternatively,
/// it can turn the Box pointer to a native pointer (*mut Mutator), then forge a mut reference from the native
/// pointer. In either way, the VM binding code needs to guarantee the safety.

/// How the VM gets mutator/collector/tracelocal handles:
/// * Mutator: from bind_mutator() as Box<Mutator>
/// * Collector: from Collection::spawn_worker_thread() as &mut Collector
/// * TraceLocal: Scanning::* as &mut TraceLocal

use std::sync::atomic::Ordering;

use crate::plan::mutator_context::{Mutator, MutatorContext};
use crate::plan::Plan;
use crate::scheduler::GCWorker;

use crate::vm::Collection;

use crate::util::{Address, ObjectReference};

use self::selected_plan::SelectedPlan;
use crate::plan::selected_plan;
use crate::util::alloc::allocators::AllocatorSelector;

use crate::mmtk::MMTK;
use crate::plan::Allocator;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::layout::vm_layout_constants::HEAP_END;
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub fn start_control_collector<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.base().control_collector_context.run(tls);
}

pub fn gc_init<VM: VMBinding>(mmtk: &'static mut MMTK<VM>, heap_size: usize) {
    crate::util::logger::init().unwrap();
    mmtk.plan.gc_init(heap_size, &mmtk.vm_map, &mmtk.scheduler);
}

pub fn bind_mutator<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    tls: OpaquePointer,
) -> Box<Mutator<SelectedPlan<VM>>> {
    SelectedPlan::bind_mutator(&mmtk.plan, tls, mmtk)
}

pub fn destroy_mutator<VM: VMBinding>(mutator: Box<Mutator<SelectedPlan<VM>>>) {
    drop(mutator);
}

pub fn flush_mutator<VM: VMBinding>(mutator: &mut Mutator<SelectedPlan<VM>>) {
    mutator.flush()
}

pub fn alloc<VM: VMBinding>(
    mutator: &mut Mutator<SelectedPlan<VM>>,
    size: usize,
    align: usize,
    offset: isize,
    allocator: Allocator,
) -> Address {
    mutator.alloc(size, align, offset, allocator)
}

pub fn post_alloc<VM: VMBinding>(
    mutator: &mut Mutator<SelectedPlan<VM>>,
    refer: ObjectReference,
    type_refer: ObjectReference,
    bytes: usize,
    allocator: Allocator,
) {
    mutator.post_alloc(refer, type_refer, bytes, allocator);
}

// Returns an AllocatorSelector for the given allocator. This method is provided so that VM compilers may call it to help generate allocation fastpath.
pub fn get_allocator_mapping<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    allocator: Allocator,
) -> AllocatorSelector {
    mmtk.plan.get_allocator_mapping()[allocator]
}

pub fn start_worker<VM: VMBinding>(
    tls: OpaquePointer,
    worker: &'static mut GCWorker<VM>,
    mmtk: &'static MMTK<VM>,
) {
    worker.init(tls);
    worker.run(mmtk);
}

pub fn enable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
    mmtk.scheduler.initialize(mmtk.options.threads, mmtk, tls);
    VM::VMCollection::spawn_worker_thread(tls, None); // spawn controller thread
    mmtk.plan.base().initialized.store(true, Ordering::SeqCst);
}

pub fn process<VM: VMBinding>(mmtk: &'static MMTK<VM>, name: &str, value: &str) -> bool {
    unsafe { mmtk.options.process(name, value) }
}

pub fn used_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_pages_used() << LOG_BYTES_IN_PAGE
}

pub fn free_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_free_pages() << LOG_BYTES_IN_PAGE
}

pub fn starting_heap_address() -> Address {
    HEAP_START
}

pub fn last_heap_address() -> Address {
    HEAP_END
}

pub fn total_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

#[cfg(feature = "sanity")]
pub fn scan_region() {
    crate::util::sanity::memory_scan::scan_region();
}

pub fn handle_user_collection_request<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.handle_user_collection_request(tls, false);
}

pub fn is_live_object(object: ObjectReference) -> bool {
    object.is_live()
}

pub fn is_mapped_object(object: ObjectReference) -> bool {
    object.is_mapped()
}

pub fn is_mapped_address(address: Address) -> bool {
    address.is_mapped()
}

pub fn modify_check<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) {
    mmtk.plan.modify_check(object);
}

pub fn add_weak_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_weak_candidate::<VM>(reff, referent);
}

pub fn add_soft_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_soft_candidate::<VM>(reff, referent);
}

pub fn add_phantom_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_phantom_candidate::<VM>(reff, referent);
}

pub fn harness_begin<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.harness_begin(tls);
}

pub fn harness_end<VM: VMBinding>(mmtk: &'static MMTK<VM>) {
    mmtk.harness_end();
}
