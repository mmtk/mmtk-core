use libc::c_void;
use libc::c_char;
use std::ffi::CStr;
use std::ptr::null_mut;
use mmtk::memory_manager;
use mmtk::Allocator;
use mmtk::util::{ObjectReference, OpaquePointer, Address};
use mmtk::Plan;
use mmtk::util::constants::LOG_BYTES_IN_PAGE;
use mmtk::{SelectedMutator, SelectedTraceLocal, SelectedCollector};

use OpenJDK;
use UPCALLS;
use OpenJDK_Upcalls;
use SINGLETON;

#[no_mangle]
pub extern "C" fn openjdk_gc_init(calls: *const OpenJDK_Upcalls, heap_size: usize) {
    unsafe { UPCALLS = calls };
    memory_manager::gc_init(&SINGLETON, heap_size);
}

#[no_mangle]
pub extern "C" fn start_control_collector(tls: OpaquePointer) {
    memory_manager::start_control_collector(&SINGLETON, tls);
}

#[no_mangle]
pub extern "C" fn bind_mutator(tls: OpaquePointer) -> *mut SelectedMutator<OpenJDK> {
    Box::into_raw(memory_manager::bind_mutator(&SINGLETON, tls))
}

#[no_mangle]
pub extern "C" fn destroy_mutator(mutator: *mut SelectedMutator<OpenJDK>) {
    memory_manager::destroy_mutator(unsafe { Box::from_raw(mutator) })
}

#[no_mangle]
pub extern "C" fn alloc(mutator: *mut SelectedMutator<OpenJDK>, size: usize,
                    align: usize, offset: isize, allocator: Allocator) -> Address {
    memory_manager::alloc::<OpenJDK>(unsafe { &mut *mutator }, size, align, offset, allocator)
}

#[no_mangle]
pub extern "C" fn alloc_slow(mutator: *mut SelectedMutator<OpenJDK>, size: usize,
                                        align: usize, offset: isize, allocator: Allocator) -> Address {
    memory_manager::alloc_slow::<OpenJDK>(unsafe { &mut *mutator }, size, align, offset, allocator)
}

#[no_mangle]
pub extern "C" fn post_alloc(mutator: *mut SelectedMutator<OpenJDK>, refer: ObjectReference, type_refer: ObjectReference,
                                        bytes: usize, allocator: Allocator) {
    memory_manager::post_alloc::<OpenJDK>(unsafe { &mut *mutator }, refer, type_refer, bytes, allocator)
}

#[no_mangle]
pub extern "C" fn will_never_move(object: ObjectReference) -> bool {
    memory_manager::will_never_move(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn is_valid_ref(val: ObjectReference) -> bool {
    memory_manager::is_valid_ref(&SINGLETON, val)
}

#[no_mangle]
pub extern "C" fn report_delayed_root_edge(trace_local: *mut SelectedTraceLocal<OpenJDK>, addr: Address) {
    memory_manager::report_delayed_root_edge(&SINGLETON, unsafe { &mut *trace_local }, addr)
}

#[no_mangle]
pub extern "C" fn will_not_move_in_current_collection(trace_local: *mut SelectedTraceLocal<OpenJDK>, obj: ObjectReference) -> bool {
    memory_manager::will_not_move_in_current_collection(&SINGLETON, unsafe { &mut *trace_local}, obj)
}

#[no_mangle]
pub extern "C" fn process_interior_edge(trace_local: *mut SelectedTraceLocal<OpenJDK>, target: ObjectReference, slot: Address, root: bool) {
    memory_manager::process_interior_edge(&SINGLETON, unsafe { &mut *trace_local }, target, slot, root)
}

#[no_mangle]
pub extern "C" fn start_worker(tls: OpaquePointer, worker: *mut SelectedCollector<OpenJDK>) {
    memory_manager::start_worker::<OpenJDK>(tls, unsafe { worker.as_mut().unwrap() })
}

#[no_mangle]
pub extern "C" fn enable_collection(tls: OpaquePointer) {
    memory_manager::enable_collection(&SINGLETON, tls)
}

#[no_mangle]
pub extern "C" fn used_bytes() -> usize {
    memory_manager::used_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn free_bytes() -> usize {
    memory_manager::free_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn total_bytes() -> usize {
    memory_manager::total_bytes(&SINGLETON)
}

#[no_mangle]
#[cfg(feature = "sanity")]
pub extern "C" fn scan_region() {
    memory_manager::scan_region(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn trace_get_forwarded_referent(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_referent::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_get_forwarded_reference(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_reference::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_is_live(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: ObjectReference) -> bool{
    memory_manager::trace_is_live::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_root_object(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: ObjectReference) -> ObjectReference {
    memory_manager::trace_root_object::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn process_edge(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: Address) {
    memory_manager::process_edge::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_retain_referent(trace_local: *mut SelectedTraceLocal<OpenJDK>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_retain_referent::<OpenJDK>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn handle_user_collection_request(tls: OpaquePointer) {
    memory_manager::handle_user_collection_request::<OpenJDK>(&SINGLETON, tls);
}

#[no_mangle]
pub extern "C" fn is_mapped_object(object: ObjectReference) -> bool {
    memory_manager::is_mapped_object(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn is_mapped_address(object: Address) -> bool {
    memory_manager::is_mapped_address(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn modify_check(object: ObjectReference) {
    memory_manager::modify_check(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn add_weak_candidate(reff: ObjectReference, referent: ObjectReference) {
    memory_manager::add_weak_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub extern "C" fn add_soft_candidate(reff: ObjectReference, referent: ObjectReference) {
    memory_manager::add_soft_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub extern "C" fn add_phantom_candidate(reff: ObjectReference, referent: ObjectReference) {
    memory_manager::add_phantom_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub extern "C" fn harness_begin(tls: OpaquePointer) {
    memory_manager::harness_begin(&SINGLETON, tls)
}

#[no_mangle]
pub extern "C" fn harness_end(tls: OpaquePointer) {
    memory_manager::harness_end(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn process(name: *const c_char, value: *const c_char) -> bool {
    let name_str: &CStr = unsafe { CStr::from_ptr(name) };
    let value_str: &CStr = unsafe { CStr::from_ptr(value) };
    memory_manager::process(&SINGLETON, name_str.to_str().unwrap(), value_str.to_str().unwrap())
}

#[no_mangle]
pub extern "C" fn starting_heap_address() -> Address {
    memory_manager::starting_heap_address()
}

#[no_mangle]
pub extern "C" fn last_heap_address() -> Address {
    memory_manager::last_heap_address()
}

#[no_mangle]
pub extern "C" fn openjdk_max_capacity() -> usize {
    SINGLETON.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

#[no_mangle]
pub extern "C" fn executable() -> bool {
    true
}
