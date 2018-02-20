use ::util::address::Address;
use super::allocator::{align_allocation, fill_alignment_gap};

use ::util::alloc::Allocator;
use ::util::heap::PageResource;

use std::marker::PhantomData;

use ::policy::space::Space;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
#[derive(Debug)]
pub struct BumpAllocator<'a, T: 'a, PR: PageResource<'a, T>> where T: Space<'a, PR> {
    pub thread_id: usize,
    cursor: Address,
    limit: Address,
    space: Option<&'a T>,
    _placeholder: PhantomData<PR>
}

impl<'a, T, PR: PageResource<'a, T>> BumpAllocator<'a, T, PR> where T: Space<'a, PR> {
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

impl<'a, T, PR: PageResource<'a, T>> Allocator<'a, T, PR> for BumpAllocator<'a, T, PR> where T: Space<'a, PR> {
    fn get_space(&self) -> Option<&'a T> {
        self.space
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let result = align_allocation(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            trace!("Thread local buffer used up, go to alloc slow path");
            self.alloc_slow(size, align, offset)
        } else {
            fill_alignment_gap(self.cursor, result);
            self.cursor = new_cursor;
            trace!("Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
                   size, result, self.cursor, self.limit);
            result
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start: Address = self.space.unwrap().acquire(self.thread_id, block_size);
        if acquired_start.is_zero() {
            trace!("Failed to acquire a new block");
            acquired_start
        } else {
            trace!("Acquired a new block of size {} with start address {}",
                   block_size, acquired_start);
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }
}

impl<'a, T, PR: PageResource<'a, T>> BumpAllocator<'a, T, PR> where T: Space<'a, PR> {
    pub fn new(thread_id: usize, space: Option<&'a T>) -> Self {
        BumpAllocator {
            thread_id,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
            _placeholder: PhantomData,
        }
    }
}
