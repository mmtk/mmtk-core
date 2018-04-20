use ::util::address::Address;
use super::allocator::{align_allocation_no_fill, fill_alignment_gap};

use ::util::alloc::Allocator;
use ::util::heap::PageResource;
use ::util::alloc::linear_scan::LinearScan;

use std::marker::PhantomData;

use libc::{memset, c_void};

use ::policy::space::Space;
use util::conversions::bytes_to_pages;
use ::util::constants::BYTES_IN_ADDRESS;


const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

const REGION_LIMIT_OFFSET: isize = 0;
const NEXT_REGION_OFFSET: isize = REGION_LIMIT_OFFSET + BYTES_IN_ADDRESS as isize;
const DATA_END_OFFSET: isize = NEXT_REGION_OFFSET + BYTES_IN_ADDRESS as isize;

#[repr(C)]
#[derive(Debug)]
pub struct BumpAllocator<S: Space<PR>, PR: PageResource<S>> where S: 'static {
    pub thread_id: usize,
    cursor: Address,
    limit: Address,
    space: Option<&'static S>,
    _placeholder: PhantomData<PR>,
}

impl<S: Space<PR>, PR: PageResource<S>> BumpAllocator<S, PR> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }

    pub fn rebind(&mut self, space: Option<&'static S>) {
        self.reset();
        self.space = space;
    }

    fn scan_region<T: LinearScan>(scanner: T, start: Address) {}
}

impl<S: Space<PR>, PR: PageResource<S>> Allocator<S, PR> for BumpAllocator<S, PR> {
    fn get_space(&self) -> Option<&'static S> {
        self.space
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc");
        let result = align_allocation_no_fill(self.cursor, align, offset);
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

impl<S: Space<PR>, PR: PageResource<S>> BumpAllocator<S, PR> {
    pub fn new(thread_id: usize, space: Option<&'static S>) -> Self {
        BumpAllocator {
            thread_id,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
            _placeholder: PhantomData,
        }
    }
}
