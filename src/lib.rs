#![cfg_attr(feature = "jikesrvm", feature(asm))]

extern crate libc;
#[macro_use]
extern crate lazy_static;

pub mod util;
pub mod vm;
mod policy;
mod plan;

use std::ptr::null_mut;
use libc::c_void;

use plan::plan::Plan;
use ::plan::mutator_context::MutatorContext;

#[cfg(feature = "jikesrvm")]
use ::vm::JTOC_BASE;

#[cfg(feature = "jikesrvm")]
use ::util::address::Address;

use ::plan::selected_plan;
use selected_plan::{SelectedPlan, SelectedMutator};

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub extern fn jikesrvm_gc_init(jtoc: *mut c_void, heap_size: usize) {
    unsafe { JTOC_BASE = Address::from_mut_ptr(jtoc); }
    selected_plan::PLAN.gc_init(heap_size);
    ::vm::scheduler::test1();
    println!("{}", ::vm::scheduler::test(44));
    println!("{}", ::vm::scheduler::test2(45, 67));
    ::vm::scheduler::test1();
    println!("{}", ::vm::scheduler::test3(21, 34, 9, 8));
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn jikesrvm_gc_init(_jtoc: *mut c_void, _heap_size: usize) {
    panic!("Cannot call jikesrvm_gc_init when not building for JikesRVM");
}

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub extern fn start_control_collector(thread_id: usize) {
    selected_plan::PLAN.control_collector_context.run(thread_id);
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn start_control_collector(rvm_thread: *mut c_void) {
    panic!("Cannot call start_control_collector when not building for JikesRVM");
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    if cfg!(feature = "jikesrvm") {
        panic!("Should be calling jikesrvm_gc_init instead");
    }
    selected_plan::PLAN.gc_init(heap_size);
}

#[no_mangle]
pub extern fn bind_mutator(thread_id: usize) -> *mut c_void {
    SelectedPlan::bind_mutator(&selected_plan::PLAN, thread_id)
}

#[no_mangle]
pub fn alloc(mutator: *mut c_void, size: usize,
             align: usize, offset: isize) -> *mut c_void {
    let local = unsafe { &mut *(mutator as *mut SelectedMutator) };
    local.alloc(size, align, offset).as_usize() as *mut c_void
}

#[no_mangle]
#[inline(never)]
pub fn alloc_slow(mutator: *mut c_void, size: usize,
                  align: usize, offset: isize) -> *mut c_void {
    let local = unsafe { &mut *(mutator as *mut SelectedMutator) };
    local.alloc_slow(size, align, offset).as_usize() as *mut c_void
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_large(_mutator: *mut c_void, _size: usize,
                          _align: usize, _offset: isize) -> *mut c_void {
    panic!("Not implemented");
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}