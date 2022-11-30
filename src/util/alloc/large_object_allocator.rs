use crate::plan::Plan;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::util::alloc::{allocator, Allocator};
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;

#[repr(C)]
pub struct LargeObjectAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    space: &'static LargeObjectSpace<VM>,
    /// [`Plan`] instance that this allocator instance is associated with.
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> Allocator<VM> for LargeObjectAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn get_space(&self) -> &'static dyn Space<VM> {
        // Casting the interior of the Option: from &LargeObjectSpace to &dyn Space
        self.space as &'static dyn Space<VM>
    }

    fn does_thread_local_allocation(&self) -> bool {
        false
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let cell: Address = self.alloc_slow(size, align, offset);
        // We may get a null ptr from alloc due to the VM being OOM
        if !cell.is_zero() {
            allocator::align_allocation::<VM>(cell, align, offset, VM::MIN_ALIGNMENT, true)
        } else {
            cell
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, _offset: isize) -> Address {
        let header = 0; // HashSet is used instead of DoublyLinkedList
        let maxbytes =
            allocator::get_maximum_aligned_size::<VM>(size + header, align, VM::MIN_ALIGNMENT);
        let pages = crate::util::conversions::bytes_to_pages_up(maxbytes);
        let sp = self.space.allocate_pages(self.tls, pages);
        if sp.is_zero() {
            sp
        } else {
            sp + header
        }
    }
}

impl<VM: VMBinding> LargeObjectAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static LargeObjectSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        LargeObjectAllocator { tls, space, plan }
    }
}
