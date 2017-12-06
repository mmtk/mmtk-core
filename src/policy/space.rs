use libc::{mmap, PROT_READ, PROT_WRITE, PROT_EXEC, MAP_PRIVATE, MAP_ANON, c_void, munmap};
use ::util::address::Address;
use std::ptr::null_mut;

use std::mem::uninitialized;

const SPACE_ALIGN: usize = 1 << 19;

#[derive(Debug)]
pub struct Space {
    mmap_start: usize,
    mmap_len: usize,
    heap_cursor: Address,
    heap_limit: Address,
}

impl Space {
    pub fn new() -> Self {
        // FIXME: We need to find a better solution here.
        //
        //       This is a workaround for what appears to be a compiler bug.
        //       Essentially, it tries to initialize multiple fields at once
        //       using a `movapd` but fails because the alignment is wrong.
        //
        //       This is (relatively) safe to do because the fields only get
        //       properly initialized in `init`, not here.
        unsafe { uninitialized() }
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
        self.mmap_len = heap_size + SPACE_ALIGN;
    }

    pub fn acquire(&mut self, size: usize) -> Address {
        let old_cursor = self.heap_cursor;
        let new_cursor = self.heap_cursor + size;
        if new_cursor > self.heap_limit {
            unsafe { Address::zero() }
        } else {
            self.heap_cursor = new_cursor;
            old_cursor
        }
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        let unmap_result = unsafe { munmap(self.mmap_start as *mut c_void, self.mmap_len) };
        if unmap_result != 0 {
            panic!("Failed to unmap {:?}", self);
        }
    }
}