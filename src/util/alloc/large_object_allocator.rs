use libc::c_void;

use ::policy::largeobjectspace::LargeObjectSpace;
use ::policy::space::Space;
use ::util::{Address, ObjectReference};
use ::util::alloc::{allocator, Allocator};
use ::util::heap::{FreeListPageResource, PageResource};
use ::util::OpaquePointer;
use ::plan::selected_plan::SelectedPlan;
use vm::VMBinding;

#[repr(C)]
pub struct LargeObjectAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static LargeObjectSpace<VM>>,
    plan: &'static SelectedPlan<VM>,
}

impl<VM: VMBinding> Allocator<VM, FreeListPageResource<VM, LargeObjectSpace<VM>>> for LargeObjectAllocator<VM> {
    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
        self.plan
    }

    fn get_space(&self) -> Option<&'static LargeObjectSpace<VM>> {
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
        let sp = self.space.unwrap().allocate_pages(self.tls, pages);
        if sp.is_zero() {
            sp
        } else {
            sp + header
        }
    }
}

impl<VM: VMBinding> LargeObjectAllocator<VM> {
    pub fn new(tls: OpaquePointer, space: Option<&'static LargeObjectSpace<VM>>, plan: &'static SelectedPlan<VM>) -> Self {
        LargeObjectAllocator {
            tls,
            space, plan,
        }
    }
}