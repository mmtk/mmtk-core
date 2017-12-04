extern crate libc;
#[macro_use]
extern crate lazy_static;

pub mod util;
mod policy;
mod plan;

use std::ptr::null_mut;
use libc::c_void;
use plan::nogc as gc_plan;

type MMTkHandle = *mut gc_plan::ThreadLocalAllocData;

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    gc_plan::init(heap_size);
}

#[no_mangle]
pub extern fn bind_allocator(thread_id: usize) -> MMTkHandle {
    gc_plan::bind_allocator(thread_id)
}

#[no_mangle]
pub extern fn alloc(handle: MMTkHandle, size: usize,
                    align: usize, offset: isize) -> *mut c_void {
    gc_plan::alloc(handle, size, align, offset)
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_slow(handle: MMTkHandle, size: usize,
                         align: usize, offset: isize) -> *mut c_void {
    gc_plan::alloc_slow(handle, size, align, offset)
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_large(_handle: MMTkHandle, _size: usize,
                          _align: usize, _offset: isize) -> *mut c_void {
    panic!("Not implemented");
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}