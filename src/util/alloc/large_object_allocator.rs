use libc::c_void;

use ::policy::largeobjectspace::LargeObjectSpace;
use ::policy::space::Space;
use ::util::{Address, ObjectReference};
use ::util::alloc::{allocator, Allocator};
use ::util::heap::{FreeListPageResource, PageResource};

#[repr(C)]
#[derive(Debug)]
pub struct LargeObjectAllocator {
    pub tls: *mut c_void,
    space: Option<&'static LargeObjectSpace>,
}

impl Allocator<FreeListPageResource<LargeObjectSpace>> for LargeObjectAllocator {
    fn get_tls(&self) -> *mut c_void {
        self.tls
    }

    fn get_space(&self) -> Option<&'static LargeObjectSpace> {
        self.space
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let cell: Address = self.alloc_slow(size, align, offset);
        allocator::align_allocation(cell, align, offset, allocator::MIN_ALIGNMENT, true)
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow_inline(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let header = 0; // HashSet is used instead of DoublyLinkedList
        let maxbytes = allocator::get_maximum_aligned_size(size + header, align, allocator::MIN_ALIGNMENT);
        let pages = ::util::conversions::bytes_to_pages_up(maxbytes);
        let sp = self.space.unwrap().acquire(self.tls, pages);
        if sp.is_zero() {
            sp
        } else {
            sp + header
        }
    }
}

impl LargeObjectAllocator {
    pub fn new(tls: *mut c_void, space: Option<&'static LargeObjectSpace>) -> Self {
        LargeObjectAllocator {
            tls,
            space,
        }
    }
}