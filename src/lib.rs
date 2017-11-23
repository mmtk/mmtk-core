extern crate memmap;
#[macro_use]
extern crate lazy_static;

mod address;

use address::{Address, ObjectReference};
use memmap::*;
use std::cell::UnsafeCell;
use std::marker::Sync;

const SPACE_ALIGN: usize = 1 << 19;

pub struct NotThreadSafe<T> {
    value: UnsafeCell<T>,
}

unsafe impl<T> Sync for NotThreadSafe<T> {}

impl<T> NotThreadSafe<T> {
    pub fn get(&self) -> *mut T {
        self.value.get()
    }
}

pub struct Space {
    heap_start: Address,
    heap_cursor: Address,
    heap_end: Address,
    address_range: MmapMut,
}

lazy_static! {
    static ref IMMORTAL_SPACE: NotThreadSafe<Option<Space>> = NotThreadSafe { value: UnsafeCell::new(None) };
}

impl Space {
    pub unsafe fn new(heap_size: usize) -> Self {
        let address_range = MmapMut::map_anon(heap_size).
            expect("Unable to allocate memory");
        let raw_start: Address = Address::from_ptr::<u8>(address_range.as_ptr());
        let heap_start: Address = raw_start.align_up(SPACE_ALIGN);

        Space {
            heap_start: heap_start,
            heap_cursor: heap_start,
            heap_end: raw_start + heap_size,
            address_range: address_range,
        }
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe { *IMMORTAL_SPACE.value.get() = Some(Space::new(heap_size)) };
}

#[inline(always)]
fn align_allocation(region: Address, align: usize, offset: usize) -> Address {
    let region_isize = region.as_usize() as isize;
    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -region_isize; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize, offset: usize) -> ObjectReference {
    let space: &mut Option<Space> = unsafe { &mut *IMMORTAL_SPACE.get() };
    let old_cursor = space.as_ref().unwrap().heap_cursor;
    let new_cursor = align_allocation(old_cursor + size, align, offset);
    if new_cursor > space.as_ref().unwrap().heap_end {
        unsafe { Address::zero().to_object_reference() }
    } else {
        space.as_mut().unwrap().heap_cursor = new_cursor;
        unsafe { old_cursor.to_object_reference() }
    }
}