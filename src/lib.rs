#![feature(asm)]

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
use util::alloc::allocator::Allocator;

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

    let res: usize;
    unsafe {
        let call_addr = (::vm::JTOC_BASE + ::vm::jtoc::TEST_METHOD_JTOC_OFFSET).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((::vm::JTOC_BASE + ::vm::jtoc::THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4)).load::<usize>();

        asm!("mov eax, 45" : : : "eax" : "intel");
        asm!("mov esi, ecx" : : "{ecx}"(rvm_thread) : "esi" : "intel");
        asm!("call ebx" : : "{ebx}"(call_addr) : "eax" : "intel");
        asm!("mov $0, eax" : "=r"(res) : : : "intel");
        asm!("sub sp, 4" : : : : "intel");
    }

    println!("{}", res);
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn jikesrvm_gc_init(_jtoc: *mut c_void, _heap_size: usize) {
    panic!("Cannot call jikesrvm_gc_init when not building for JikesRVM");
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