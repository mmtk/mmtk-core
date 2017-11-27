extern crate libc;

mod address;

use address::Address;
use libc::*;
use std::ptr::null_mut;
use std::marker::Sync;

const SPACE_ALIGN: usize = 1 << 19;

pub struct VeryUnsafeCell<T: ? Sized> {
    value: T,
}

unsafe impl<T> Sync for VeryUnsafeCell<T> {}

impl<T> VeryUnsafeCell<T> {
    #[inline(always)]
    pub fn new(value: T) -> VeryUnsafeCell<T> {
        VeryUnsafeCell { value }
    }
    #[inline(always)]
    pub unsafe fn into_inner(self) -> T {
        self.value
    }
    #[inline(always)]
    pub fn get(&self) -> *mut T {
        &self.value as *const T as *mut T
    }
}

pub struct Space {
    mmap_start: *mut c_void,
    heap_start: Address,
    heap_cursor: Address,
    heap_end: Address,
}

static mut IMMORTAL_SPACE: VeryUnsafeCell<Space> = VeryUnsafeCell {
    value:
    Space {
        mmap_start: 0 as *mut c_void,
        heap_start: Address(0),
        heap_cursor: Address(0),
        heap_end: Address(0),
    }
};

impl Space {
    pub fn init(&mut self, heap_size: usize) {
        self.mmap_start = unsafe {
            mmap(null_mut(), heap_size + SPACE_ALIGN, PROT_READ | PROT_WRITE | PROT_EXEC,
                 MAP_PRIVATE | MAP_ANON, -1, 0)
        };

        self.heap_start = Address::from_ptr::<c_void>(self.mmap_start).align_up(SPACE_ALIGN);

        self.heap_cursor = self.heap_start;
        self.heap_end = self.heap_start + heap_size;
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe {
        (*IMMORTAL_SPACE.get()).init(heap_size);
    }
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
pub extern fn alloc(size: usize, align: usize, offset: isize) -> *mut c_void {
    let space: &mut Space = unsafe { &mut *IMMORTAL_SPACE.get() };
    let result = align_allocation(space.heap_cursor, align, offset);
    let new_cursor = result + size;
    if new_cursor > space.heap_end {
        unsafe { Address::zero().to_object_reference().value() as *mut c_void }
    } else {
        space.heap_cursor = new_cursor;
        unsafe { result.to_object_reference().value() as *mut c_void }
    }
}

#[no_mangle]
pub extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(size, 1, 0)
}

#[no_mangle]
pub extern fn mmtk_free(ptr: *const c_void) {}