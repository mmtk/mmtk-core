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
        let mut ret = Space {
            heap_start: Address::zero(),
            heap_cursor: Address::zero(),
            heap_end: Address::zero(),
            address_range: MmapMut::map_anon(heap_size).unwrap(),
        };

        ret.heap_start = Address::from_ptr::<u8>(ret.address_range.as_ptr())
            .align_up(SPACE_ALIGN);
        ret.heap_cursor = ret.heap_start;
        ret.heap_end = ret.heap_start + heap_size;

        ret
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe {
        *IMMORTAL_SPACE.write().unwrap() = Some(Space::new(heap_size));
    }
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize) -> ObjectReference {
    println!("Allocating");
    let mut space = IMMORTAL_SPACE.write().unwrap();
    let old_cursor = space.as_ref().unwrap().heap_cursor;
    let new_cursor = (old_cursor + size).align_up(align);
    if new_cursor > space.as_ref().unwrap().heap_end {
        println!("GC is triggered when GC is disabled");
        unsafe { Address::zero().to_object_reference() }
    } else {
        space.as_mut().unwrap().heap_cursor = new_cursor;
        println!("Allocated");
        unsafe { old_cursor.to_object_reference() }
    }
}