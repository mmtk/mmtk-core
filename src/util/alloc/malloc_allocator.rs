use crate::policy::space::Space;
use crate::policy::mallocspace::MallocSpace;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::plan::global::Plan;

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
        self.alloc_slow(size, align, offset)
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // TODO: We currently ignore the offset field. This is wrong.
        // assert!(offset == 0);
        assert!(align <= 16);
        let ret = self.space.unwrap().alloc(size);
        trace!("MallocSpace.alloc size = {}, align = {}, offset = {}, res = {}", size, align, offset, ret);
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
