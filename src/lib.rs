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
        let address_range = MmapMut::map_anon(heap_size + SPACE_ALIGN).unwrap();
        let heap_start = Address::from_ptr::<u8>(address_range.as_ptr())
            .align_up(SPACE_ALIGN);

        Space {
            heap_start: heap_start,
            heap_cursor: heap_start,
            heap_end: heap_start + heap_size,
            address_range: address_range,
        }
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe { *IMMORTAL_SPACE.value.get() = Some(Space::new(heap_size)) };
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize) -> ObjectReference {
    let space: &mut Option<Space> = unsafe { &mut *IMMORTAL_SPACE.get() };
    let old_cursor = space.as_ref().unwrap().heap_cursor;
    let new_cursor = (old_cursor + size).align_up(align);
    if new_cursor > space.as_ref().unwrap().heap_end {
        println!("Run out of heap space");
        unsafe { Address::zero().to_object_reference() }
    } else {
        space.as_mut().unwrap().heap_cursor = new_cursor;
        unsafe { old_cursor.to_object_reference() }
    }
}