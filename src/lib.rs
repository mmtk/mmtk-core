extern crate memmap;
#[macro_use]
extern crate lazy_static;

mod address;

use address::Address;
use memmap::*;

use std::sync::RwLock;

const SPACE_ALIGN: usize = 1 << 19;

pub struct Space {
    heap_start:    Address,
    heap_cursor:   Address,
    heap_end:      Address,
    address_range: MmapMut,
}

lazy_static! {
    pub static ref immortal_space: RwLock<Option<Space>> = RwLock::new(None);
}

impl Space {
    pub unsafe fn new(heap_size: usize) -> Self {
        let mut ret = Space {
            heap_start:    Address::zero(),
            heap_cursor:   Address::zero(),
            heap_end:      Address::zero(),
            address_range: MmapOptions::new().len(heap_size).map_anon().unwrap(),
        };

        ret.heap_start  = Address::from_ptr::<u8>(ret.address_range.as_ptr())
            .align_up(SPACE_ALIGN);
        ret.heap_cursor = ret.heap_start;
        ret.heap_end    = ret.heap_start.plus(heap_size);

        ret
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe {
        *immortal_space.write().unwrap() = Some(Space::new(heap_size));
    }
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize) -> Address {
    unsafe {Address::zero()}
}