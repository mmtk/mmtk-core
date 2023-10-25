use std::sync::Arc;

use crate::policy::marksweepspace::malloc_ms::MallocSpace;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::rust_util::flex_mut::ArcFlexMut;
use crate::util::Address;
use crate::vm::VMBinding;

use super::allocator::AllocatorContext;

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    space: ArcFlexMut<MallocSpace<VM>>,
    context: Arc<AllocatorContext<VM>>,
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_context(&self) -> &AllocatorContext<VM> {
        &self.context
    }

    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        self.alloc_slow(size, align, offset)
    }

    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn does_thread_local_allocation(&self) -> bool {
        false
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        self.space
            .read()
            .alloc(self.tls, size, align, offset)
            .unwrap_or_else(|_| {
                use crate::vm::Collection;
                VM::VMCollection::block_for_gc(VMMutatorThread(self.tls));
                Address::ZERO
            })
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub(crate) fn new(
        tls: VMThread,
        space: ArcFlexMut<MallocSpace<VM>>,
        context: Arc<AllocatorContext<VM>>,
    ) -> Self {
        MallocAllocator {
            tls,
            space,
            context,
        }
    }
}
