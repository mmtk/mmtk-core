use ::util::address::Address;

use ::util::alloc::allocator::align_allocation;
use ::util::alloc::Allocator;

use ::policy::space::Space;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
#[derive(Debug)]
pub struct BumpAllocator<'a, T: 'a> where T: Space {
    pub thread_id: usize,
    cursor: Address,
    limit: Address,
    space: Option<&'a T>,
}

impl<'a, T> BumpAllocator<'a, T> where T: Space {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }

    pub fn rebind(&mut self, space: Option<&'a T>) {
        self.reset();
        self.space = space;
    }
}

impl<'a, T> Allocator<'a, T> for BumpAllocator<'a, T> where T: Space {
    fn get_space(&self) -> Option<&'a T> {
        self.space
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
        let acquired_start: Address = self.space.unwrap().acquire(self.thread_id, block_size);
        if acquired_start.is_zero() {
            acquired_start
        } else {
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }
}

impl<'a, T> BumpAllocator<'a, T> where T: Space {
    pub fn new(thread_id: usize, space: Option<&'a T>) -> Self {
        BumpAllocator {
            thread_id,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
        }
    }
}
