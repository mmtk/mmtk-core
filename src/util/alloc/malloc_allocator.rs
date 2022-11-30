use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;
use crate::Plan;

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    space: &'static MallocSpace<VM>,
    /// [`Plan`] instance that this allocator instance is associated with.
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space as &'static dyn Space<VM>
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow(size, align, offset)
    }

    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn does_thread_local_allocation(&self) -> bool {
        false
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        assert!(offset >= 0);

        self.space.alloc(self.tls, size, align, offset)
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static MallocSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
