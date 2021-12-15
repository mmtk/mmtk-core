// All functions here are extern function. There is no point for marking them as unsafe.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use libc::c_char;
use std::ffi::CStr;
use mmtk::memory_manager;
use mmtk::AllocationSemantics;
use mmtk::util::{ObjectReference, Address};
use mmtk::util::opaque_pointer::*;
use mmtk::scheduler::GCWorker;
use mmtk::Mutator;
use mmtk::MMTK;
use DummyVM;
use SINGLETON;

#[no_mangle]
pub extern "C" fn gc_init(heap_size: usize) {
    // # Safety
    // Casting `SINGLETON` as mutable is safe because `gc_init` will only be executed once by a single thread during startup.
    #[allow(clippy::cast_ref_to_mut)]
    let singleton_mut = unsafe { &mut *(&*SINGLETON as *const MMTK<DummyVM> as *mut MMTK<DummyVM>) };
    memory_manager::gc_init(singleton_mut, heap_size)
}

#[no_mangle]
pub extern "C" fn start_control_collector(tls: VMWorkerThread) {
    memory_manager::start_control_collector(&SINGLETON, tls);
}

#[no_mangle]
pub extern "C" fn bind_mutator(tls: VMMutatorThread) -> *mut Mutator<DummyVM> {
    Box::into_raw(memory_manager::bind_mutator(&SINGLETON, tls))
}

#[no_mangle]
pub extern "C" fn destroy_mutator(mutator: *mut Mutator<DummyVM>) {
    memory_manager::destroy_mutator(unsafe { Box::from_raw(mutator) })
}

#[no_mangle]
pub extern "C" fn alloc(mutator: *mut Mutator<DummyVM>, size: usize,
                    align: usize, offset: isize, mut semantics: AllocationSemantics) -> Address {
    if size >= SINGLETON.get_plan().constraints().max_non_los_default_alloc_bytes {
        semantics = AllocationSemantics::Los;
    }
    memory_manager::alloc::<DummyVM>(unsafe { &mut *mutator }, size, align, offset, semantics)
}

#[no_mangle]
pub extern "C" fn post_alloc(mutator: *mut Mutator<DummyVM>, refer: ObjectReference,
                                        bytes: usize, mut semantics: AllocationSemantics) {
    if bytes >= SINGLETON.get_plan().constraints().max_non_los_default_alloc_bytes {
        semantics = AllocationSemantics::Los;
    }
    memory_manager::post_alloc::<DummyVM>(unsafe { &mut *mutator }, refer, bytes, semantics)
}

#[no_mangle]
pub extern "C" fn will_never_move(object: ObjectReference) -> bool {
    !object.is_movable()
}

#[no_mangle]
pub extern "C" fn start_worker(tls: VMWorkerThread, worker: &'static mut GCWorker<DummyVM>, mmtk: &'static MMTK<DummyVM>) {
    memory_manager::start_worker::<DummyVM>(tls, worker, mmtk)
}

#[no_mangle]
pub extern "C" fn initialize_collection(tls: VMThread) {
    memory_manager::initialize_collection(&SINGLETON, tls)
}

#[no_mangle]
pub extern "C" fn disable_collection() {
    memory_manager::disable_collection(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn enable_collection() {
    memory_manager::enable_collection(&SINGLETON)
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
pub extern "C" fn is_live_object(object: ObjectReference) -> bool{
    object.is_live()
}

#[no_mangle]
pub extern "C" fn is_mapped_object(object: ObjectReference) -> bool {
    object.is_mapped()
}

#[no_mangle]
pub extern "C" fn is_mapped_address(address: Address) -> bool {
    address.is_mapped()
}

#[no_mangle]
pub extern "C" fn modify_check(object: ObjectReference) {
    memory_manager::modify_check(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn handle_user_collection_request(tls: VMMutatorThread) {
    memory_manager::handle_user_collection_request::<DummyVM>(&SINGLETON, tls);
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
pub extern "C" fn harness_begin(tls: VMMutatorThread) {
    memory_manager::harness_begin(&SINGLETON, tls)
}

#[no_mangle]
pub extern "C" fn harness_end() {
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
