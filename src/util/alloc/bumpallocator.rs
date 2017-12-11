use std::sync::Mutex;

use ::util::heap::MonotonePageResource;
use ::util::heap::PageResource;
use ::util::address::Address;

use ::util::alloc::allocator::align_allocation;
use ::util::alloc::allocator::Allocator;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
#[derive(Debug)]
pub struct BumpAllocator<'a> {
    thread_id: usize,
    cursor: Address,
    limit: Address,
    space: &'a Mutex<MonotonePageResource>,
}

impl<'a> BumpAllocator<'a> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }
}

impl<'a> Allocator<'a> for BumpAllocator<'a> {
    fn new(thread_id: usize, space: &'a Mutex<MonotonePageResource>) -> Self {
        BumpAllocator {
            thread_id,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let result = align_allocation(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            self.alloc_slow(size, align, offset)
        } else {
            self.cursor = new_cursor;
            result
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let mut space = self.space.lock().unwrap();
        let acquired_start: Address = (*space).get_new_pages(block_size);
        if acquired_start.is_zero() {
            acquired_start
        } else {
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }
}
