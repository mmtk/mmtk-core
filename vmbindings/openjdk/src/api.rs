use libc::c_void;
use libc::c_char;
use std::ptr::null_mut;
use mmtk::memory_manager;
use mmtk::Allocator;
use mmtk::util::{ObjectReference, OpaquePointer, Address};
use mmtk::Plan;
use mmtk::util::constants::LOG_BYTES_IN_PAGE;

use OpenJDK;
use UPCALLS;
use OpenJDK_Upcalls;
use SINGLETON;

#[no_mangle]
pub unsafe extern fn openjdk_gc_init(calls: *const OpenJDK_Upcalls, heap_size: usize) {
    UPCALLS = calls;
    memory_manager::gc_init(&SINGLETON, heap_size);
}

#[no_mangle]
pub extern fn start_control_collector(tls: OpaquePointer) {
    memory_manager::start_control_collector(&SINGLETON, tls);
}

#[no_mangle]
pub extern fn bind_mutator(tls: OpaquePointer) -> *mut c_void {
    memory_manager::bind_mutator(&SINGLETON, tls)
}

#[no_mangle]
pub unsafe extern fn alloc(mutator: *mut c_void, size: usize,
                    align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    memory_manager::alloc::<OpenJDK>(mutator, size, align, offset, allocator)
}

#[no_mangle]
pub unsafe extern fn alloc_slow(mutator: *mut c_void, size: usize,
                                        align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    memory_manager::alloc_slow::<OpenJDK>(mutator, size, align, offset, allocator)
}

#[no_mangle]
pub unsafe extern fn post_alloc(mutator: *mut c_void, refer: ObjectReference, type_refer: ObjectReference,
                                        bytes: usize, allocator: Allocator) {
    memory_manager::post_alloc::<OpenJDK>(mutator, refer, type_refer, bytes, allocator)
}

//#[no_mangle]
//pub unsafe extern fn mmtk_malloc(size: usize) -> *mut c_void {
//    memory_manager::mmtk_malloc::<OpenJDK>(size)
//}

#[no_mangle]
pub extern fn will_never_move(object: ObjectReference) -> bool {
    memory_manager::will_never_move(&SINGLETON, object)
}

#[no_mangle]
pub extern fn is_valid_ref(val: ObjectReference) -> bool {
    memory_manager::is_valid_ref(&SINGLETON, val)
}

#[no_mangle]
pub unsafe extern fn report_delayed_root_edge(trace_local: *mut c_void, addr: *mut c_void) {
    memory_manager::report_delayed_root_edge(&SINGLETON, trace_local, addr)
}

#[no_mangle]
pub unsafe extern fn will_not_move_in_current_collection(trace_local: *mut c_void, obj: *mut c_void) -> bool {
    memory_manager::will_not_move_in_current_collection(&SINGLETON, trace_local, obj)
}

#[no_mangle]
pub unsafe extern fn process_interior_edge(trace_local: *mut c_void, target: *mut c_void, slot: *mut c_void, root: bool) {
    memory_manager::process_interior_edge(&SINGLETON, trace_local, target, slot, root)
}

#[no_mangle]
pub unsafe extern fn start_worker(tls: OpaquePointer, worker: *mut c_void) {
    memory_manager::start_worker::<OpenJDK>(tls, worker)
}

#[no_mangle]
pub extern fn enable_collection(tls: OpaquePointer) {
    memory_manager::enable_collection(&SINGLETON, tls)
}

#[no_mangle]
pub extern fn used_bytes() -> usize {
    memory_manager::used_bytes(&SINGLETON)
}

#[no_mangle]
pub extern fn free_bytes() -> usize {
    memory_manager::free_bytes(&SINGLETON)
}

#[no_mangle]
pub extern fn total_bytes() -> usize {
    memory_manager::total_bytes(&SINGLETON)
}

#[no_mangle]
#[cfg(feature = "sanity")]
pub extern fn scan_region() {
    memory_manager::scan_region(&SINGLETON)
}

#[no_mangle]
pub unsafe extern fn trace_get_forwarded_referent(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_referent::<OpenJDK>(trace_local, object)
}

#[no_mangle]
pub unsafe extern fn trace_get_forwarded_reference(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_reference::<OpenJDK>(trace_local, object)
}

#[no_mangle]
pub unsafe extern fn trace_is_live(trace_local: *mut c_void, object: ObjectReference) -> bool{
    memory_manager::trace_is_live::<OpenJDK>(trace_local, object)
}

#[no_mangle]
pub unsafe extern fn trace_retain_referent(trace_local: *mut c_void, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_retain_referent::<OpenJDK>(trace_local, object)
}

#[no_mangle]
pub extern fn handle_user_collection_request(tls: OpaquePointer) {
    memory_manager::handle_user_collection_request::<OpenJDK>(&SINGLETON, tls);
}

#[no_mangle]
pub extern fn is_mapped_object(object: ObjectReference) -> bool {
    memory_manager::is_mapped_object(&SINGLETON, object)
}

#[no_mangle]
pub extern fn is_mapped_address(object: Address) -> bool {
    memory_manager::is_mapped_address(&SINGLETON, object)
}

#[no_mangle]
pub extern fn modify_check(object: ObjectReference) {
    memory_manager::modify_check(&SINGLETON, object)
}

#[no_mangle]
pub unsafe extern fn add_weak_candidate(reff: *mut c_void, referent: *mut c_void) {
    memory_manager::add_weak_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub unsafe extern fn add_soft_candidate(reff: *mut c_void, referent: *mut c_void) {
    memory_manager::add_soft_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub unsafe extern fn add_phantom_candidate(reff: *mut c_void, referent: *mut c_void) {
    memory_manager::add_phantom_candidate(&SINGLETON, reff, referent)
}

#[no_mangle]
pub extern fn harness_begin(tls: OpaquePointer) {
    memory_manager::harness_begin(&SINGLETON, tls)
}

#[no_mangle]
pub extern fn harness_end(tls: OpaquePointer) {
    memory_manager::harness_end(&SINGLETON)
}

#[no_mangle]
pub extern fn process(name: *const c_char, value: *const c_char) -> bool {
    memory_manager::process(&SINGLETON, name, value)
}

#[no_mangle]
pub extern fn starting_heap_address() -> *mut c_void {
    memory_manager::starting_heap_address()
}

#[no_mangle]
pub extern fn last_heap_address() -> *mut c_void {
    memory_manager::last_heap_address()
}

#[no_mangle]
pub extern fn openjdk_max_capacity() -> usize {
    SINGLETON.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

#[no_mangle]
pub extern fn executable() -> bool {
    true
}
