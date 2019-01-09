use ::util::{Address, ObjectReference};
use super::allocator::{align_allocation_no_fill, fill_alignment_gap, MIN_ALIGNMENT};

use ::util::alloc::Allocator;
use ::util::heap::{PageResource, FreeListPageResource};
use ::util::alloc::linear_scan::LinearScan;
use ::util::alloc::dump_linear_scan::DumpLinearScan;
use policy::regionspace::*;

use ::vm::ObjectModel;
use ::vm::VMObjectModel;

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

type PR = FreeListPageResource<RegionSpace>;

#[repr(C)]
#[derive(Debug)]
pub struct RegionAllocator {
    pub tls: *mut c_void,
    cursor: Address,
    limit: Address,
    pub space: &'static RegionSpace,
}

impl RegionAllocator {
    pub fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }
}

impl Allocator<PR> for RegionAllocator {
    fn get_space(&self) -> Option<&'static RegionSpace> {
        Some(self.space)
    }

    fn alloc(&mut self, bytes: usize, align: usize, offset: isize) -> Address {
        debug_assert!(bytes <= BYTES_IN_REGION);
        trace!("alloc");
        let start = align_allocation_no_fill(self.cursor, align, offset);
        let end = start + bytes;
        // check whether we've exceeded the limit
        if end > self.limit {
            return self.alloc_slow(bytes, align, offset);
        }
        // sufficient memory is available, so we can finish performing the allocation
        fill_alignment_gap(self.cursor, start);
        self.cursor = end;
        Region::of(start).cursor = end;
        start
    }

    fn alloc_slow(&mut self, bytes: usize, align: usize, offset: isize) -> Address {
        debug_assert!(bytes <= BYTES_IN_REGION);
        // TODO: internalLimit etc.
        self.alloc_slow_inline(bytes, align, offset)
    }

    fn alloc_slow_once(&mut self, bytes: usize, align: usize, offset: isize) -> Address {
        trace!("alloc_slow");
        match self.space.acquire_new_region(self.tls) {
            Some(region) => {
                self.cursor = region.0;
                self.limit = self.cursor + BYTES_IN_REGION;
                self.alloc(bytes, align, offset)
            },
            None => unsafe { Address::zero() },
        }
    }

    fn get_tls(&self) -> *mut c_void {
        self.tls
    }
}

impl RegionAllocator {
    pub fn new(tls: *mut c_void, space: &'static RegionSpace) -> Self {
        RegionAllocator {
            tls,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
        }
    }
}
