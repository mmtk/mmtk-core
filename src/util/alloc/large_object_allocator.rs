use std::sync::Arc;

use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::policy::space_ref::SpaceRef;
use crate::util::alloc::{allocator, Allocator};
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;

use super::allocator::AllocatorContext;

#[repr(C)]
pub struct LargeObjectAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    space: SpaceRef<LargeObjectSpace<VM>>,
    context: Arc<AllocatorContext<VM>>,
    _pad: usize,
}

impl<VM: VMBinding> Allocator<VM> for LargeObjectAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_context(&self) -> &AllocatorContext<VM> {
        &self.context
    }

    fn does_thread_local_allocation(&self) -> bool {
        false
    }

    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        let cell: Address = self.alloc_slow(size, align, offset);
        // We may get a null ptr from alloc due to the VM being OOM
        if !cell.is_zero() {
            allocator::align_allocation::<VM>(cell, align, offset)
        } else {
            cell
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, _offset: usize) -> Address {
        if crate::space_ref_read!(&self.space).will_oom_on_acquire(self.tls, size) {
            return Address::ZERO;
        }

        let maxbytes = allocator::get_maximum_aligned_size::<VM>(size, align);
        let pages = crate::util::conversions::bytes_to_pages_up(maxbytes);
        let alloc_res = crate::space_ref_read!(&self.space).allocate_pages(self.tls, pages);
        match alloc_res {
            Ok(addr) => addr,
            Err(_) => {
                use crate::vm::Collection;
                VM::VMCollection::block_for_gc(VMMutatorThread(self.tls));
                Address::ZERO
            }
        }
    }
}

impl<VM: VMBinding> LargeObjectAllocator<VM> {
    pub(crate) fn new(
        tls: VMThread,
        space: SpaceRef<LargeObjectSpace<VM>>,
        context: Arc<AllocatorContext<VM>>,
    ) -> Self {
        LargeObjectAllocator {
            tls,
            space,
            context,
            _pad: 0,
        }
    }
}
