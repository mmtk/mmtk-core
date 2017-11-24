extern crate memmap;
#[macro_use]
extern crate lazy_static;

mod address;

use address::{Address, ObjectReference};
use memmap::*;
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
    heap_start: Address,
    heap_cursor: Address,
    heap_end: Address,
    address_range: Option<MmapMut>,
    // we do not access this in fast path
}

lazy_static! {
    static ref IMMORTAL_SPACE: VeryUnsafeCell<Space> = VeryUnsafeCell {value: Space::new() };
}

impl Space {
    pub fn new() -> Self {
        unsafe {
            Space {
                heap_start: Address::zero(),
                heap_cursor: Address::zero(),
                heap_end: Address::zero(),
                address_range: None,
            }
        }
    }

    pub fn init(&mut self, heap_size: usize) {
        let address_range = MmapMut::map_anon(heap_size + SPACE_ALIGN).
            expect("Unable to allocate memory");

        self.heap_start = Address::from_ptr::<u8>(address_range.as_ptr())
            .align_up(SPACE_ALIGN);

        self.heap_cursor = self.heap_start;
        self.heap_end = self.heap_start + heap_size;

        self.address_range = Some(address_range);
    }
}

#[no_mangle]
pub extern fn gc_init(heap_size: usize) {
    unsafe {
        (*IMMORTAL_SPACE.get()).init(heap_size);
    }
}

#[inline(always)]
fn align_allocation(region: Address, align: usize, offset: usize) -> Address {
    let region_isize = region.as_usize() as isize;
    let offset_isize = offset as isize;

    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -offset_isize; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize, offset: usize) -> ObjectReference {
    let space: &mut Space = unsafe { &mut *IMMORTAL_SPACE.get() };
    let result = align_allocation(space.heap_cursor, align, offset);
    let new_cursor = result + size;
    if new_cursor > space.heap_end {
        unsafe { Address::zero().to_object_reference() }
    } else {
        space.heap_cursor = new_cursor;
        unsafe { result.to_object_reference() }
    }
}