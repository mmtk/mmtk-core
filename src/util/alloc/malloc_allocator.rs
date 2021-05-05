use crate::plan::global::Plan;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MallocSpace<VM>,
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

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // TODO: We currently ignore the offset field. This is wrong.
        // assert!(offset == 0);
        assert!(align <= 16);
        let ret = self.space.alloc(self.tls, size);
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
        tls: VMThread,
        space: &'static MallocSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
