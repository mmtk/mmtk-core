use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::plan::transitive_closure::TransitiveClosure;
use crate::plan::CollectorContext;
use crate::plan::MutatorContext;
use crate::plan::Plan;
use crate::plan::TraceLocal;
use crate::scheduler::GCWorker;

use crate::vm::Collection;

use crate::util::{Address, ObjectReference};

use self::selected_plan::SelectedPlan;
use crate::plan::selected_plan;

use self::selected_plan::{SelectedCollector, SelectedMutator, SelectedTraceLocal};
use crate::mmtk::MMTK;
use crate::plan::Allocator;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::layout::vm_layout_constants::HEAP_END;
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::scheduler::Scheduler;

// This file provides a safe Rust API for mmtk-core.
// We expect the VM binding to inherit and extend this API by:
// 1. adding their VM-specific functions
// 2. exposing the functions to native if necessary. And the VM binding needs to manage the unsafety
//    for exposing this safe API to FFI.

// For example, for mutators, this API provides a Box<Mutator>, and requires a &mut Mutator for allocation.
// A VM binding can borrow a mutable reference directly from Box<Mutator>, and call alloc(). Alternatively,
// it can turn the Box pointer to a native pointer (*mut Mutator), then forge a mut reference from the native
// pointer. In either way, the VM binding code needs to guarantee the safety.

// How the VM gets mutator/collector/tracelocal handles:
// * Mutator: from bind_mutator() as Box<Mutator>
// * Collector: from Collection::spawn_worker_thread() as &mut Collector
// * TraceLocal: Scanning::* as &mut TraceLocal

pub fn start_control_collector<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.base().control_collector_context.as_ref().unwrap().run(tls);
}

pub fn gc_init<VM: VMBinding>(mmtk: &MMTK<VM>, heap_size: usize) {
    crate::util::logger::init().unwrap();
    let mmtk_mut = unsafe { &mut *(mmtk as *const _ as *mut MMTK<VM>) };
    mmtk_mut.plan.gc_init(heap_size, unsafe { &*(mmtk as *const _) });

    // TODO: We should have an option so we know whether we should spawn the controller.
    //    thread::spawn(|| {
    //        SINGLETON.plan.common().control_collector_context.run(UNINITIALIZED_OPAQUE_POINTER )
    //    });
}

pub fn bind_mutator<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    tls: OpaquePointer,
) -> Box<SelectedMutator<VM>> {
    SelectedPlan::bind_mutator(&mmtk.plan, tls)
}

pub fn destroy_mutator<VM: VMBinding>(mutator: Box<SelectedMutator<VM>>) {
    drop(mutator);
}

pub fn alloc<VM: VMBinding>(
    mutator: &mut SelectedMutator<VM>,
    size: usize,
    align: usize,
    offset: isize,
    allocator: Allocator,
) -> Address {
    mutator.alloc(size, align, offset, allocator)
}

pub fn post_alloc<VM: VMBinding>(
    mutator: &mut SelectedMutator<VM>,
    refer: ObjectReference,
    type_refer: ObjectReference,
    bytes: usize,
    allocator: Allocator,
) {
    mutator.post_alloc(refer, type_refer, bytes, allocator);
}

// The parameter 'trace_local' could either be &mut SelectedTraceLocal or &mut SanityChecker.
// Ideally we should make 'trace_local' as a trait object - &mut TraceLocal. However, this is a fat
// pointer, and it would appear in our API (and possibly in native API), which imposes inconvenience
// to store and pass around a fat pointer. Thus, we just assume it is &mut SelectedTraceLocal,
// and use unsafe transmute when we know it is a SanityChcker ref.
#[cfg(feature = "sanity")]
pub fn report_delayed_root_edge<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    addr: Address,
) {
    use crate::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.is_in_sanity() {
        let sanity_checker: &mut SanityChecker<VM> =
            unsafe { &mut *(trace_local as *mut SelectedTraceLocal<VM> as *mut SanityChecker<VM>) };
        sanity_checker.report_delayed_root_edge(addr);
    } else {
        trace_local.report_delayed_root_edge(addr)
    }
}
#[cfg(not(feature = "sanity"))]
pub fn report_delayed_root_edge<VM: VMBinding>(
    _: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    addr: Address,
) {
    trace_local.report_delayed_root_edge(addr);
}

#[cfg(feature = "sanity")]
pub fn will_not_move_in_current_collection<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    obj: ObjectReference,
) -> bool {
    use crate::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.is_in_sanity() {
        let sanity_checker: &mut SanityChecker<VM> =
            unsafe { &mut *(trace_local as *mut SelectedTraceLocal<VM> as *mut SanityChecker<VM>) };
        sanity_checker.will_not_move_in_current_collection(obj)
    } else {
        trace_local.will_not_move_in_current_collection(obj)
    }
}
#[cfg(not(feature = "sanity"))]
pub fn will_not_move_in_current_collection<VM: VMBinding>(
    _: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    obj: ObjectReference,
) -> bool {
    trace_local.will_not_move_in_current_collection(obj)
}

#[cfg(feature = "sanity")]
pub fn process_interior_edge<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    target: ObjectReference,
    slot: Address,
    root: bool,
) {
    use crate::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.is_in_sanity() {
        let sanity_checker: &mut SanityChecker<VM> =
            unsafe { &mut *(trace_local as *mut SelectedTraceLocal<VM> as *mut SanityChecker<VM>) };
        sanity_checker.process_interior_edge(target, slot, root)
    } else {
        trace_local.process_interior_edge(target, slot, root)
    }
}
#[cfg(not(feature = "sanity"))]
pub fn process_interior_edge<VM: VMBinding>(
    _: &MMTK<VM>,
    trace_local: &mut SelectedTraceLocal<VM>,
    target: ObjectReference,
    slot: Address,
    root: bool,
) {
    trace_local.process_interior_edge(target, slot, root)
}

pub fn start_worker<VM: VMBinding>(tls: OpaquePointer, worker: &'static mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
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

pub fn trace_get_forwarded_referent<VM: VMBinding>(
    trace_local: &mut SelectedTraceLocal<VM>,
    object: ObjectReference,
) -> ObjectReference {
    trace_local.get_forwarded_reference(object)
}

pub fn trace_get_forwarded_reference<VM: VMBinding>(
    trace_local: &mut SelectedTraceLocal<VM>,
    object: ObjectReference,
) -> ObjectReference {
    trace_local.get_forwarded_reference(object)
}

pub fn trace_root_object<VM: VMBinding>(
    trace_local: &mut SelectedTraceLocal<VM>,
    object: ObjectReference,
) -> ObjectReference {
    trace_local.trace_object(object)
}

pub extern "C" fn process_edge<VM: VMBinding>(
    trace_local: &mut SelectedTraceLocal<VM>,
    object: Address,
) {
    trace_local.process_edge(object);
}

pub fn trace_retain_referent<VM: VMBinding>(
    trace_local: &mut SelectedTraceLocal<VM>,
    object: ObjectReference,
) -> ObjectReference {
    trace_local.retain_referent(object)
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

pub fn harness_end<VM: VMBinding>(mmtk: &MMTK<VM>) {
    mmtk.harness_end();
}
