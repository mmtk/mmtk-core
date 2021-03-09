use crate::plan::global::Plan;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::allocator;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static MallocSpace<VM>>,
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space.map(|s| s as &'static dyn Space<VM>)
    }
    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let cell = self.alloc_slow(size, align, offset);
        allocator::align_allocation::<VM>(cell, align, offset, VM::MIN_ALIGNMENT, true)
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let maxbytes = allocator::get_maximum_aligned_size::<VM>(size, align, VM::MIN_ALIGNMENT);
        let ret = self.space.unwrap().alloc(maxbytes);
        trace!(
            "MallocSpace.alloc size = {}, align = {}, offset = {}, res = {}",
            size,
            align,
            offset,
            ret
        );
        ret
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static MallocSpace<VM>>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
