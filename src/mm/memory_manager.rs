use std::ptr::null_mut;
use libc::c_void;
use libc::c_char;

use std::ffi::CStr;
use std::{str, thread};

use std::sync::atomic::Ordering;

use plan::Plan;
use ::plan::MutatorContext;
use ::plan::TraceLocal;
use ::plan::CollectorContext;
use ::plan::ParallelCollectorGroup;

use ::vm::Collection;

use ::util::{Address, ObjectReference};

use ::plan::selected_plan;
use self::selected_plan::SelectedPlan;

use ::plan::Allocator;
use util::constants::LOG_BYTES_IN_PAGE;
use util::heap::layout::vm_layout_constants::HEAP_START;
use util::heap::layout::vm_layout_constants::HEAP_END;
use util::OpaquePointer;
use crate::mmtk::SINGLETON;
use vm::VMBinding;
use mmtk::MMTK;

pub fn start_control_collector<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.common().control_collector_context.run(tls);
}

pub fn gc_init<VM: VMBinding>(mmtk: &MMTK<VM>, heap_size: usize) {
    ::util::logger::init().unwrap();
    mmtk.plan.gc_init(heap_size, &SINGLETON.vm_map);
    mmtk.plan.common().initialized.store(true, Ordering::SeqCst);

    // TODO: We should have an option so we know whether we should spawn the controller.
//    thread::spawn(|| {
//        SINGLETON.plan.common().control_collector_context.run(UNINITIALIZED_OPAQUE_POINTER )
//    });
}

pub fn bind_mutator<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) -> *mut c_void {
    SelectedPlan::bind_mutator(&SINGLETON.plan, tls)
}

pub unsafe fn alloc<VM: VMBinding>(mutator: *mut c_void, size: usize,
             align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    let local = &mut *(mutator as *mut <SelectedPlan<VM> as Plan<VM>>::MutatorT);
    local.alloc(size, align, offset, allocator).to_mut_ptr()
}

#[inline(never)]
pub unsafe fn alloc_slow<VM: VMBinding>(mutator: *mut c_void, size: usize,
                  align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    let local = &mut *(mutator as *mut <SelectedPlan<VM> as Plan<VM>>::MutatorT);
    local.alloc_slow(size, align, offset, allocator).to_mut_ptr()
}

pub fn post_alloc<VM: VMBinding>(mutator: *mut c_void, refer: ObjectReference, type_refer: ObjectReference,
                         bytes: usize, allocator: Allocator) {
    let local = unsafe {&mut *(mutator as *mut <SelectedPlan<VM> as Plan<VM>>::MutatorT)};
    local.post_alloc(refer, type_refer, bytes, allocator);
}

// TODO: I dont think this works. The null pointer will get de-referenced.
//pub unsafe fn mmtk_malloc<VM: VMBinding>(size: usize) -> *mut c_void {
//    alloc::<VM>(null_mut(), size, 1, 0, Allocator::Default)
//}

//#[no_mangle]
//pub extern fn mmtk_free(_ptr: *const c_void) {}

pub fn will_never_move<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) -> bool {
    mmtk.plan.will_never_move(object)
}

pub fn is_valid_ref<VM: VMBinding>(mmtk: &MMTK<VM>, val: ObjectReference) -> bool {
    mmtk.plan.is_valid_ref(val)
}

#[cfg(feature = "sanity")]
pub unsafe fn report_delayed_root_edge<VM: VMBinding>(mmtk: &MMTK<VM>, trace_local: *mut c_void, addr: *mut c_void) {
    use ::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.common().is_in_sanity() {
        report_delayed_root_edge_inner::<SanityChecker<VM>>(trace_local, addr)
    } else {
        report_delayed_root_edge_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, addr)
    }
}
#[cfg(not(feature = "sanity"))]
pub unsafe fn report_delayed_root_edge<VM: VMBinding>(_: &MMTK<VM>, trace_local: *mut c_void, addr: *mut c_void) {
    report_delayed_root_edge_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, addr)
}
unsafe fn report_delayed_root_edge_inner<T: TraceLocal>(trace_local: *mut c_void, addr: *mut c_void) {
    trace!("report_delayed_root_edge with trace_local={:?}", trace_local);
    let local = &mut *(trace_local as *mut T);
    local.report_delayed_root_edge(Address::from_mut_ptr(addr));
    trace!("report_delayed_root_edge returned with trace_local={:?}", trace_local);
}

#[cfg(feature = "sanity")]
pub unsafe fn will_not_move_in_current_collection<VM: VMBinding>(mmtk: &MMTK<VM>, trace_local: *mut c_void, obj: *mut c_void) -> bool {
    use ::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.common().is_in_sanity() {
        will_not_move_in_current_collection_inner::<SanityChecker<VM>>(trace_local, obj)
    } else {
        will_not_move_in_current_collection_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, obj)
    }
}
#[cfg(not(feature = "sanity"))]
pub unsafe fn will_not_move_in_current_collection<VM: VMBinding>(_: &MMTK<VM>, trace_local: *mut c_void, obj: *mut c_void) -> bool {
    will_not_move_in_current_collection_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, obj)
}
unsafe fn will_not_move_in_current_collection_inner<T: TraceLocal>(trace_local: *mut c_void, obj: *mut c_void) -> bool {
    trace!("will_not_move_in_current_collection({:?}, {:?})", trace_local, obj);
    let local = &mut *(trace_local as *mut T);
    let ret = local.will_not_move_in_current_collection(Address::from_mut_ptr(obj).to_object_reference());
    trace!("will_not_move_in_current_collection returned with trace_local={:?}", trace_local);
    ret
}

#[cfg(feature = "sanity")]
pub unsafe fn process_interior_edge<VM: VMBinding>(mmtk: &MMTK<VM>, trace_local: *mut c_void, target: *mut c_void, slot: *mut c_void, root: bool) {
    use ::util::sanity::sanity_checker::SanityChecker;
    if mmtk.plan.common().is_in_sanity() {
        process_interior_edge_inner::<SanityChecker<VM>>(trace_local, target, slot, root)
    } else {
        process_interior_edge_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, target, slot, root)
    }
    trace!("process_interior_root_edge returned with trace_local={:?}", trace_local);
}
#[cfg(not(feature = "sanity"))]
pub unsafe fn process_interior_edge<VM: VMBinding>(_: &MMTK<VM>, trace_local: *mut c_void, target: *mut c_void, slot: *mut c_void, root: bool) {
    process_interior_edge_inner::<<SelectedPlan<VM> as Plan<VM>>::TraceLocalT>(trace_local, target, slot, root)
}
unsafe fn process_interior_edge_inner<T: TraceLocal>(trace_local: *mut c_void, target: *mut c_void, slot: *mut c_void, root: bool) {
    trace!("process_interior_edge with trace_local={:?}", trace_local);
    let local = &mut *(trace_local as *mut T);
    local.process_interior_edge(Address::from_mut_ptr(target).to_object_reference(),
                                Address::from_mut_ptr(slot), root);
    trace!("process_interior_root_edge returned with trace_local={:?}", trace_local);
}

pub unsafe fn start_worker<VM: VMBinding>(tls: OpaquePointer, worker: *mut c_void) {
    let worker_instance = &mut *(worker as *mut <SelectedPlan<VM> as Plan<VM>>::CollectorT);
    worker_instance.init(tls);
    worker_instance.run(tls);
}

pub fn enable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
    unsafe { (&mut *mmtk.plan.common().control_collector_context.workers.get()) }.init_group(mmtk, tls);
    unsafe { VM::VMCollection::spawn_worker_thread::<<SelectedPlan<VM> as Plan<VM>>::CollectorT>(tls, null_mut()); }// spawn controller thread
    mmtk.plan.common().initialized.store(true, Ordering::SeqCst);
}

#[no_mangle]
pub extern fn process(name: *const c_char, value: *const c_char) -> bool {
    let name_str: &CStr = unsafe { CStr::from_ptr(name) };
    let value_str: &CStr = unsafe { CStr::from_ptr(value) };
    let option = &SINGLETON.options;
    unsafe {
        option.process(name_str.to_str().unwrap(), value_str.to_str().unwrap())
    }
}

pub fn used_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_pages_used() << LOG_BYTES_IN_PAGE
}

pub fn free_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    SINGLETON.plan.get_free_pages() << LOG_BYTES_IN_PAGE
}

#[no_mangle]
pub extern fn starting_heap_address() -> *mut c_void {
    HEAP_START.to_mut_ptr()
}

#[no_mangle]
pub extern fn last_heap_address() -> *mut c_void {
    HEAP_END.to_mut_ptr()
}

pub fn total_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

#[cfg(feature = "sanity")]
pub fn scan_region<VM: VMBinding>(mmtk: &MMTK<VM>){
    ::util::sanity::memory_scan::scan_region(&mmtk.plan);
}

pub unsafe fn trace_get_forwarded_referent<VM: VMBinding>(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    let local = &mut *(trace_local as *mut <SelectedPlan<VM> as Plan<VM>>::TraceLocalT);
    local.get_forwarded_reference(object)
}

pub unsafe fn trace_get_forwarded_reference<VM: VMBinding>(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    let local = &mut *(trace_local as *mut <SelectedPlan<VM> as Plan<VM>>::TraceLocalT);
    local.get_forwarded_reference(object)
}

pub unsafe fn trace_is_live<VM: VMBinding>(trace_local: *mut c_void, object: ObjectReference) -> bool{
    let local = &mut *(trace_local as *mut <SelectedPlan<VM> as Plan<VM>>::TraceLocalT);
    local.is_live(object)
}

pub unsafe fn trace_retain_referent<VM: VMBinding>(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    let local = &mut *(trace_local as *mut <SelectedPlan<VM> as Plan<VM>>::TraceLocalT);
    local.retain_referent(object)
}

pub fn handle_user_collection_request<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.handle_user_collection_request(tls, false);
}

pub fn is_mapped_object<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) -> bool {
    mmtk.plan.is_mapped_object(object)
}

pub fn is_mapped_address<VM: VMBinding>(mmtk: &MMTK<VM>, address: Address) -> bool {
    mmtk.plan.is_mapped_address(address)
}

pub fn modify_check<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) {
    mmtk.plan.modify_check(object);
}

pub unsafe fn add_weak_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: *mut c_void, referent: *mut c_void) {
    mmtk.reference_processors.add_weak_candidate::<VM>(
        Address::from_mut_ptr(reff).to_object_reference(),
        Address::from_mut_ptr(referent).to_object_reference());
}

pub unsafe fn add_soft_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: *mut c_void, referent: *mut c_void) {
    mmtk.reference_processors.add_soft_candidate::<VM>(
        Address::from_mut_ptr(reff).to_object_reference(),
        Address::from_mut_ptr(referent).to_object_reference());
}

pub unsafe fn add_phantom_candidate<VM: VMBinding>(mmtk: &MMTK<VM>, reff: *mut c_void, referent: *mut c_void) {
    mmtk.reference_processors.add_phantom_candidate::<VM>(
        Address::from_mut_ptr(reff).to_object_reference(),
        Address::from_mut_ptr(referent).to_object_reference());
}

pub fn harness_begin<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.harness_begin(tls);
}

pub fn harness_end<VM: VMBinding>(mmtk: &MMTK<VM>) {
    mmtk.harness_end();
}
