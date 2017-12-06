extern crate libc;
#[macro_use]
extern crate lazy_static;

pub mod util;
mod policy;
mod plan;

use std::ptr::null_mut;
use libc::c_void;

use ::util::alloc::allocator::Allocator;
use plan::nogc as gc_plan;

type MMTkHandle<'a> = *mut gc_plan::SelectedAllocator<'a>;

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    let mut globl = gc_plan::SPACE.lock().unwrap();
    (*globl).init(heap_size);
}

#[no_mangle]
pub extern fn bind_mutator(thread_id: usize) -> MMTkHandle<'static> {
    Box::into_raw(Box::new(gc_plan::SelectedAllocator::new(thread_id, &gc_plan::SPACE)))
}

#[no_mangle]
pub fn alloc(mutator: MMTkHandle, size: usize,
             align: usize, offset: isize) -> *mut c_void {
    let local : &mut gc_plan::SelectedAllocator = unsafe { &mut *mutator };
    local.alloc(size, align, offset).as_usize() as *mut c_void
}

#[no_mangle]
#[inline(never)]
pub fn alloc_slow(mutator: MMTkHandle, size: usize,
                  align: usize, offset: isize) -> *mut c_void {
    let local: &mut gc_plan::SelectedAllocator = unsafe { &mut *mutator };
    local.alloc_slow(size, align, offset).as_usize() as *mut c_void
}

#[no_mangle]
#[inline(never)]
pub extern fn alloc_large(_mutator: MMTkHandle, _size: usize,
                          _align: usize, _offset: isize) -> *mut c_void {
    panic!("Not implemented");
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}