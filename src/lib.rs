extern crate memmap;
#[macro_use]
extern crate lazy_static;

mod address;

use address::{Address, ObjectReference};
use memmap::*;

use std::sync::RwLock;

const SPACE_ALIGN: usize = 1 << 19;

pub struct Space {
    heap_start: Address,
    heap_cursor: Address,
    heap_end: Address,
    address_range: MmapMut,
}

lazy_static! {
    pub static ref IMMORTAL_SPACE: RwLock<Option<Space>> = RwLock::new(None);
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
    unsafe {
        *IMMORTAL_SPACE.write().unwrap() = Some(Space::new(heap_size));
    }
}

fn align_allocation(region: Address, align: usize, offset: usize) -> Address {
    let region_isize = region.as_usize() as isize;
    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -region_isize; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize, offset: usize) -> ObjectReference {
    println!("Allocating");
    let mut space = IMMORTAL_SPACE.write().unwrap();
    let old_cursor = space.as_ref().unwrap().heap_cursor;
    let new_cursor = align_allocation(old_cursor + size, align, offset);
    if new_cursor > space.as_ref().unwrap().heap_end {
        println!("GC is triggered when GC is disabled");
        unsafe { Address::zero().to_object_reference() }
    } else {
        space.as_mut().unwrap().heap_cursor = new_cursor;
        println!("Allocated");
        unsafe { old_cursor.to_object_reference() }
    }
}