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

type SelectedPlan = ::plan::nogc::NoGC;
type SelectedMutator<'a> = ::plan::nogc::NoGCMutator<'a>;

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    SelectedPlan::gc_init(heap_size);
}

#[no_mangle]
pub extern fn bind_mutator(thread_id: usize) -> *mut c_void {
    SelectedPlan::bind_mutator(thread_id)
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