use ::util::address::Address;
use super::allocator::{align_allocation, fill_alignment_gap};

use ::util::alloc::Allocator;
use ::util::heap::PageResource;

use std::marker::PhantomData;

use libc::{memset, c_void};

use ::policy::space::Space;
use util::conversions::bytes_to_pages;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
#[derive(Debug)]
pub struct BumpAllocator<PR: PageResource> {
    pub thread_id: usize,
    cursor: Address,
    limit: Address,
    space: Option<&'static PR::Space>
}

impl<PR: PageResource> BumpAllocator<PR> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }

    pub fn rebind(&mut self, space: Option<&'static PR::Space>) {
        self.reset();
        self.space = space;
    }
}

impl<PR: PageResource> Allocator<PR> for BumpAllocator<PR> {
    fn get_space(&self) -> Option<&'static PR::Space> {
        self.space
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc");
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
        // TODO: internalLimit etc.
        self.alloc_slow_inline(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc_slow");
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start: Address = self.space.unwrap().acquire(self.thread_id,
                                                                  bytes_to_pages(block_size));
        if acquired_start.is_zero() {
            trace!("Failed to acquire a new block");
            acquired_start
        } else {
            trace!("Acquired a new block of size {} with start address {}",
                   block_size, acquired_start);
            unsafe {
                memset(acquired_start.as_usize() as *mut c_void, 0, block_size);
            }
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }

    fn get_thread_id(&self) -> usize {
        self.thread_id
    }
}

impl<PR: PageResource> BumpAllocator<PR> {
    pub fn new(thread_id: usize, space: Option<&'static PR::Space>) -> Self {
        BumpAllocator {
            thread_id,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
        }
    }
}
