extern crate libc;
use libc::*;

#[macro_use]
extern crate lazy_static;

pub mod address;
use address::Address;

use std::ptr::null_mut;

use std::sync::{Mutex};

const SPACE_ALIGN: usize = 1 << 19;

type MMTkHandle = *mut ThreadLocalAllocData;

pub struct ThreadLocalAllocData {
    thread_id: usize,
    cursor: Address,
    limit: Address,
}

pub struct Space {
    mmap_start: usize,
    heap_cursor: Address,
    heap_limit: Address,
}

lazy_static! {
    static ref IMMORTAL_SPACE: Mutex<Space> = Mutex::new(Space::new());
}

impl Space {
    pub fn new() -> Self {
        Space {
            mmap_start: 0,
            heap_cursor: unsafe { Address::zero() },
            heap_limit: unsafe { Address::zero() },
        }
    }

    pub fn init(&mut self, heap_size: usize) {
        let mmap_start = unsafe {
            mmap(null_mut(), heap_size + SPACE_ALIGN, PROT_READ | PROT_WRITE | PROT_EXEC,
                 MAP_PRIVATE | MAP_ANON, -1, 0)
        };

        self.heap_cursor = Address::from_ptr::<c_void>(mmap_start)
            .align_up(SPACE_ALIGN);

        self.heap_limit = self.heap_cursor + heap_size;

        self.mmap_start = mmap_start as usize;
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
        let mut globl = IMMORTAL_SPACE.lock().unwrap();
        (*globl).init(heap_size);
}

#[inline(always)]
fn align_allocation(region: Address, align: usize, offset: isize) -> Address {
    let region_isize = region.as_usize() as isize;
    let offset_isize = offset as isize;

    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -offset_isize; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

#[no_mangle]
pub extern fn bind_allocator(thread_id: usize) -> MMTkHandle {
    Box::into_raw(Box::new(ThreadLocalAllocData {
        thread_id: thread_id,
        cursor: unsafe { Address::zero() },
        limit: unsafe { Address::zero() },
    }))
}

#[no_mangle]
pub extern fn alloc(handle: MMTkHandle, size: usize,
                    align: usize, offset: isize) -> *mut c_void {
    let local = unsafe { &mut *handle };
    let result = align_allocation(local.cursor, align, offset);
    let new_cursor = result + size;

    if new_cursor > local.limit {
        alloc_slow(handle, size, align, offset)
    } else {
        local.cursor = new_cursor;
        result.as_usize() as *mut c_void
    }
}

#[no_mangle] #[inline(never)]
pub extern fn alloc_slow(handle: MMTkHandle, size: usize,
                          align: usize, offset: isize) -> *mut c_void {
    let space = IMMORTAL_SPACE.lock().unwrap();
    panic!("Not implemented");
}

#[no_mangle] #[inline(never)]
pub extern fn alloc_large(handle: MMTkHandle, size: usize,
                         align: usize, offset: isize) -> *mut c_void {
    panic!("Not implemented");
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}