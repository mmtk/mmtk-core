use crate::plan::selected_plan::SelectedPlan;
#[cfg(feature = "largeobjectspace")]
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::util::alloc::{allocator, Allocator};
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
#[cfg(feature = "largeobjectspace")]
#[repr(C)]
pub struct LargeObjectAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static LargeObjectSpace<VM>>,
    plan: &'static SelectedPlan<VM>,
}
#[cfg(feature = "largeobjectspace")]
impl<VM: VMBinding> Allocator<VM> for LargeObjectAllocator<VM> {
    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
        self.plan
    }

    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        // Casting the interior of the Option: from &LargeObjectSpace to &dyn Space
        self.space.map(|s| s as &'static dyn Space<VM>)
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let cell: Address = self.alloc_slow(size, align, offset);
        allocator::align_allocation::<VM>(cell, align, offset, VM::MIN_ALIGNMENT, true)
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow_inline(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, _offset: isize) -> Address {
        let header = 0; // HashSet is used instead of DoublyLinkedList
        let maxbytes =
            allocator::get_maximum_aligned_size::<VM>(size + header, align, VM::MIN_ALIGNMENT);
        let pages = crate::util::conversions::bytes_to_pages_up(maxbytes);
        let sp = self.space.unwrap().allocate_pages(self.tls, pages);
        if sp.is_zero() {
            sp
        } else {
            sp + header
        }
    }
}
#[cfg(feature = "largeobjectspace")]
impl<VM: VMBinding> LargeObjectAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static LargeObjectSpace<VM>>,
        plan: &'static SelectedPlan<VM>,
    ) -> Self {
        LargeObjectAllocator { tls, space, plan }
    }
}
