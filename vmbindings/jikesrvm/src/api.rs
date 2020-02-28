use libc::c_void;
use libc::c_char;
use std::ffi::CStr;
use mmtk::memory_manager;
use mmtk::util::{Address, OpaquePointer, ObjectReference};
use mmtk::Allocator;
use mmtk::{SelectedMutator, SelectedTraceLocal, SelectedCollector};
use mmtk::Plan;
use JikesRVM;
use JTOC_BASE;
use SINGLETON;
use collection::BOOT_THREAD;
use collection::VMCollection;

#[no_mangle]
pub extern "C" fn jikesrvm_gc_init(jtoc: *mut c_void, heap_size: usize) {
    unsafe {
        JTOC_BASE = Address::from_mut_ptr(jtoc);
        BOOT_THREAD
            = OpaquePointer::from_address(VMCollection::thread_from_id(1));
    }
    memory_manager::gc_init(&SINGLETON, heap_size);
    debug_assert!(54 == JikesRVM::test(44));
    debug_assert!(112 == JikesRVM::test2(45, 67));
    debug_assert!(731 == JikesRVM::test3(21, 34, 9, 8));
}

#[no_mangle]
pub extern "C" fn start_control_collector(tls: OpaquePointer) {
    memory_manager::start_control_collector(&SINGLETON, tls);
}

#[no_mangle]
pub extern "C" fn bind_mutator(tls: OpaquePointer) -> *mut SelectedMutator<JikesRVM> {
    Box::into_raw(memory_manager::bind_mutator(&SINGLETON, tls))
}

#[no_mangle]
pub extern "C" fn destroy_mutator(mutator: *mut SelectedMutator<JikesRVM>) {
    memory_manager::destroy_mutator(unsafe { Box::from_raw(mutator) })
}

#[no_mangle]
pub extern "C" fn alloc(mutator: *mut SelectedMutator<JikesRVM>, size: usize,
                           align: usize, offset: isize, allocator: Allocator) -> Address {
    memory_manager::alloc::<JikesRVM>(unsafe { &mut *mutator }, size, align, offset, allocator)
}

#[no_mangle]
pub extern "C" fn alloc_slow(mutator: *mut SelectedMutator<JikesRVM>, size: usize,
                                align: usize, offset: isize, allocator: Allocator) -> Address {
    memory_manager::alloc_slow::<JikesRVM>(unsafe { &mut *mutator }, size, align, offset, allocator)
}

#[no_mangle]
pub extern "C" fn post_alloc(mutator: *mut SelectedMutator<JikesRVM>, refer: ObjectReference, type_refer: ObjectReference,
                                bytes: usize, allocator: Allocator) {
    memory_manager::post_alloc::<JikesRVM>(unsafe { &mut *mutator }, refer, type_refer, bytes, allocator)
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
pub extern "C" fn report_delayed_root_edge(trace_local: *mut SelectedTraceLocal<JikesRVM>, addr: Address) {
    memory_manager::report_delayed_root_edge(&SINGLETON, unsafe { &mut *trace_local }, addr)
}

#[no_mangle]
pub extern "C" fn will_not_move_in_current_collection(trace_local: *mut SelectedTraceLocal<JikesRVM>, obj: ObjectReference) -> bool {
    memory_manager::will_not_move_in_current_collection(&SINGLETON, unsafe { &mut *trace_local }, obj)
}

#[no_mangle]
pub extern "C" fn process_interior_edge(trace_local: *mut SelectedTraceLocal<JikesRVM>, target: ObjectReference, slot: Address, root: bool) {
    memory_manager::process_interior_edge(&SINGLETON, unsafe { &mut *trace_local }, target, slot, root)
}

#[no_mangle]
pub extern "C" fn start_worker(tls: OpaquePointer, worker: *mut SelectedCollector<JikesRVM>) {
    memory_manager::start_worker::<JikesRVM>(tls, unsafe { worker.as_mut().unwrap() })
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
pub extern "C" fn trace_get_forwarded_referent(trace_local: *mut SelectedTraceLocal<JikesRVM>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_referent::<JikesRVM>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_get_forwarded_reference(trace_local: *mut SelectedTraceLocal<JikesRVM>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_get_forwarded_reference::<JikesRVM>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_is_live(trace_local: *mut SelectedTraceLocal<JikesRVM>, object: ObjectReference) -> bool{
    memory_manager::trace_is_live::<JikesRVM>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn trace_retain_referent(trace_local: *mut SelectedTraceLocal<JikesRVM>, object: ObjectReference) -> ObjectReference{
    memory_manager::trace_retain_referent::<JikesRVM>(unsafe { &mut *trace_local }, object)
}

#[no_mangle]
pub extern "C" fn handle_user_collection_request(tls: OpaquePointer) {
    memory_manager::handle_user_collection_request::<JikesRVM>(&SINGLETON, tls);
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

// Test
// TODO: we should remove this?

#[no_mangle]
pub extern "C" fn test_stack_alignment() {
    info!("Entering stack alignment test with no args passed");
    unsafe {
        asm!("movaps %xmm1, (%esp)" : : : "sp", "%xmm1", "memory");
    }
    info!("Exiting stack alignment test");
}

#[no_mangle]
pub extern "C" fn test_stack_alignment1(a: usize, b: usize, c: usize, d: usize, e: usize) -> usize {
    info!("Entering stack alignment test");
    info!("a:{}, b:{}, c:{}, d:{}, e:{}",
          a, b, c, d, e);
    unsafe {
        asm!("movaps %xmm1, (%esp)" : : : "sp", "%xmm1", "memory");
    }
    let result = a + b * 2 + c * 3  + d * 4 + e * 5;
    info!("Exiting stack alignment test");
    result
}
