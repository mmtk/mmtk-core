// All functions here are extern function. There is no point for marking them as unsafe.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::mmtk;
use crate::DummyVM;
use crate::SINGLETON;
use libc::c_char;
use mmtk::memory_manager;
use mmtk::scheduler::GCWorker;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::AllocationSemantics;
use mmtk::MMTKBuilder;
use mmtk::Mutator;
use std::ffi::CStr;

// This file exposes MMTk Rust API to the native code. This is not an exhaustive list of all the APIs.
// Most commonly used APIs are listed in https://docs.mmtk.io/api/mmtk/memory_manager/index.html. The binding can expose them here.

#[no_mangle]
pub extern "C" fn mmtk_create_builder() -> *mut MMTKBuilder {
    Box::into_raw(Box::new(mmtk::MMTKBuilder::new()))
}

#[no_mangle]
pub extern "C" fn mmtk_set_option_from_string(
    builder: *mut MMTKBuilder,
    name: *const c_char,
    value: *const c_char,
) -> bool {
    let builder = unsafe { &mut *builder };
    let name_str: &CStr = unsafe { CStr::from_ptr(name) };
    let value_str: &CStr = unsafe { CStr::from_ptr(value) };
    builder.set_option(name_str.to_str().unwrap(), value_str.to_str().unwrap())
}

#[no_mangle]
pub extern "C" fn mmtk_set_fixed_heap_size(builder: *mut MMTKBuilder, heap_size: usize) -> bool {
    let builder = unsafe { &mut *builder };
    builder
        .options
        .gc_trigger
        .set(mmtk::util::options::GCTriggerSelector::FixedHeapSize(
            heap_size,
        ))
}

#[no_mangle]
pub fn mmtk_init(builder: *mut MMTKBuilder) {
    let builder = unsafe { Box::from_raw(builder) };

    // Create MMTK instance.
    let mmtk = memory_manager::mmtk_init::<DummyVM>(&builder);

    // Set SINGLETON to the instance.
    SINGLETON.set(mmtk).unwrap_or_else(|_| {
        panic!("Failed to set SINGLETON");
    });
}

#[no_mangle]
pub extern "C" fn mmtk_bind_mutator(tls: VMMutatorThread) -> *mut Mutator<DummyVM> {
    Box::into_raw(memory_manager::bind_mutator(mmtk(), tls))
}

#[no_mangle]
pub extern "C" fn mmtk_destroy_mutator(mutator: *mut Mutator<DummyVM>) {
    // notify mmtk-core about destroyed mutator
    memory_manager::destroy_mutator(unsafe { &mut *mutator });
    // turn the ptr back to a box, and let Rust properly reclaim it
    let _ = unsafe { Box::from_raw(mutator) };
}

#[no_mangle]
pub extern "C" fn mmtk_alloc(
    mutator: *mut Mutator<DummyVM>,
    size: usize,
    align: usize,
    offset: usize,
    mut semantics: AllocationSemantics,
) -> Address {
    // This just demonstrates that the binding should check against `max_non_los_default_alloc_bytes` to allocate large objects.
    // In pratice, a binding may want to lift this code to somewhere in the runtime where the allocated bytes is constant so
    // they can statically know if a normal allocation or a large object allocation is needed.
    if size
        >= mmtk()
            .get_plan()
            .constraints()
            .max_non_los_default_alloc_bytes
    {
        semantics = AllocationSemantics::Los;
    }
    memory_manager::alloc::<DummyVM>(unsafe { &mut *mutator }, size, align, offset, semantics)
}

#[no_mangle]
pub extern "C" fn mmtk_post_alloc(
    mutator: *mut Mutator<DummyVM>,
    refer: ObjectReference,
    bytes: usize,
    mut semantics: AllocationSemantics,
) {
    // This just demonstrates that the binding should check against `max_non_los_default_alloc_bytes` to allocate large objects.
    // In pratice, a binding may want to lift this code to somewhere in the runtime where the allocated bytes is constant so
    // they can statically know if a normal allocation or a large object allocation is needed.
    if bytes
        >= mmtk()
            .get_plan()
            .constraints()
            .max_non_los_default_alloc_bytes
    {
        semantics = AllocationSemantics::Los;
    }
    memory_manager::post_alloc::<DummyVM>(unsafe { &mut *mutator }, refer, bytes, semantics)
}

#[no_mangle]
pub extern "C" fn mmtk_start_worker(tls: VMWorkerThread, worker: *mut GCWorker<DummyVM>) {
    let worker = unsafe { Box::from_raw(worker) };
    memory_manager::start_worker::<DummyVM>(mmtk(), tls, worker)
}

#[no_mangle]
pub extern "C" fn mmtk_initialize_collection(tls: VMThread) {
    memory_manager::initialize_collection(mmtk(), tls)
}

#[no_mangle]
pub extern "C" fn mmtk_used_bytes() -> usize {
    memory_manager::used_bytes(mmtk())
}

#[no_mangle]
pub extern "C" fn mmtk_free_bytes() -> usize {
    memory_manager::free_bytes(mmtk())
}

#[no_mangle]
pub extern "C" fn mmtk_total_bytes() -> usize {
    memory_manager::total_bytes(mmtk())
}

#[no_mangle]
pub extern "C" fn mmtk_is_live_object(object: ObjectReference) -> bool {
    memory_manager::is_live_object(object)
}

#[no_mangle]
pub extern "C" fn mmtk_will_never_move(object: ObjectReference) -> bool {
    !object.is_movable()
}

#[cfg(feature = "is_mmtk_object")]
#[no_mangle]
pub extern "C" fn mmtk_is_mmtk_object(addr: Address) -> bool {
    memory_manager::is_mmtk_object(addr).is_some()
}

#[no_mangle]
pub extern "C" fn mmtk_is_in_mmtk_spaces(object: ObjectReference) -> bool {
    memory_manager::is_in_mmtk_spaces(object)
}

#[no_mangle]
pub extern "C" fn mmtk_is_mapped_address(address: Address) -> bool {
    memory_manager::is_mapped_address(address)
}

#[no_mangle]
pub extern "C" fn mmtk_handle_user_collection_request(tls: VMMutatorThread) {
    memory_manager::handle_user_collection_request::<DummyVM>(mmtk(), tls);
}

#[no_mangle]
pub extern "C" fn mmtk_add_weak_candidate(reff: ObjectReference) {
    memory_manager::add_weak_candidate(mmtk(), reff)
}

#[no_mangle]
pub extern "C" fn mmtk_add_soft_candidate(reff: ObjectReference) {
    memory_manager::add_soft_candidate(mmtk(), reff)
}

#[no_mangle]
pub extern "C" fn mmtk_add_phantom_candidate(reff: ObjectReference) {
    memory_manager::add_phantom_candidate(mmtk(), reff)
}

#[no_mangle]
pub extern "C" fn mmtk_harness_begin(tls: VMMutatorThread) {
    memory_manager::harness_begin(mmtk(), tls)
}

#[no_mangle]
pub extern "C" fn mmtk_harness_end() {
    memory_manager::harness_end(mmtk())
}

#[no_mangle]
pub extern "C" fn mmtk_starting_heap_address() -> Address {
    memory_manager::starting_heap_address()
}

#[no_mangle]
pub extern "C" fn mmtk_last_heap_address() -> Address {
    memory_manager::last_heap_address()
}

#[no_mangle]
#[cfg(feature = "malloc_counted_size")]
pub extern "C" fn mmtk_counted_malloc(size: usize) -> Address {
    memory_manager::counted_malloc::<DummyVM>(mmtk(), size)
}
#[no_mangle]
pub extern "C" fn mmtk_malloc(size: usize) -> Address {
    memory_manager::malloc(size)
}

#[no_mangle]
#[cfg(feature = "malloc_counted_size")]
pub extern "C" fn mmtk_counted_calloc(num: usize, size: usize) -> Address {
    memory_manager::counted_calloc::<DummyVM>(mmtk(), num, size)
}
#[no_mangle]
pub extern "C" fn mmtk_calloc(num: usize, size: usize) -> Address {
    memory_manager::calloc(num, size)
}

#[no_mangle]
#[cfg(feature = "malloc_counted_size")]
pub extern "C" fn mmtk_realloc_with_old_size(
    addr: Address,
    size: usize,
    old_size: usize,
) -> Address {
    memory_manager::realloc_with_old_size::<DummyVM>(mmtk(), addr, size, old_size)
}
#[no_mangle]
pub extern "C" fn mmtk_realloc(addr: Address, size: usize) -> Address {
    memory_manager::realloc(addr, size)
}

#[no_mangle]
#[cfg(feature = "malloc_counted_size")]
pub extern "C" fn mmtk_free_with_size(addr: Address, old_size: usize) {
    memory_manager::free_with_size::<DummyVM>(mmtk(), addr, old_size)
}
#[no_mangle]
pub extern "C" fn mmtk_free(addr: Address) {
    memory_manager::free(addr)
}

#[no_mangle]
#[cfg(feature = "malloc_counted_size")]
pub extern "C" fn mmtk_get_malloc_bytes() -> usize {
    memory_manager::get_malloc_bytes(mmtk())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn mmtk_init_test() {
        // We demonstrate the main workflow to initialize MMTk, create mutators and allocate objects.
        let builder = mmtk_create_builder();

        // Set option by value using extern "C" wrapper.
        let success = mmtk_set_fixed_heap_size(builder, 1048576);
        assert!(success);

        // Set option by value.  We set the the option direcly using `MMTKOption::set`. Useful if
        // the VM binding wants to set options directly, or if the VM binding has its own format for
        // command line arguments.
        let name = CString::new("plan").unwrap();
        let val = CString::new("NoGC").unwrap();
        let success = mmtk_set_option_from_string(builder, name.as_ptr(), val.as_ptr());
        assert!(success);

        // Set layout if necessary
        // builder.set_vm_layout(layout);

        // Init MMTk
        mmtk_init(builder);

        // Create an MMTk mutator
        let tls = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED)); // FIXME: Use the actual thread pointer or identifier
        let mutator = mmtk_bind_mutator(tls);

        // Do an allocation
        let addr = mmtk_alloc(mutator, 16, 8, 0, mmtk::AllocationSemantics::Default);
        assert!(!addr.is_zero());

        // Turn the allocation address into the object reference.
        let obj = DummyVM::object_start_to_ref(addr);

        // Post allocation
        mmtk_post_alloc(mutator, obj, 16, mmtk::AllocationSemantics::Default);

        // If the thread quits, destroy the mutator.
        mmtk_destroy_mutator(mutator);
    }
}
