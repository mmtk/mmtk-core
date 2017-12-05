extern crate libc;
#[macro_use]
extern crate lazy_static;

pub mod util;
mod policy;
mod plan;

use std::ptr::null_mut;
use libc::c_void;
use plan::nogc as gc_plan;

type Mutator = *mut gc_plan::NoGCMutator;

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    gc_plan::init(heap_size);
}

#[no_mangle]
pub extern fn bind_mutator(thread_id: usize) -> Mutator {
    gc_plan::bind_mutator(thread_id)
}

#[no_mangle]
pub extern fn alloc(mutator: Mutator, size: usize,
                    align: usize, offset: isize) -> *mut c_void {
    gc_plan::alloc(mutator, size, align, offset)
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_slow(mutator: Mutator, size: usize,
                         align: usize, offset: isize) -> *mut c_void {
    gc_plan::alloc_slow(mutator, size, align, offset)
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_large(_mutator: Mutator, _size: usize,
                          _align: usize, _offset: isize) -> *mut c_void {
    panic!("Not implemented");
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}