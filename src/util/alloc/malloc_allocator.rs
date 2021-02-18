use crate::policy::space::Space;
use crate::policy::mallocspace::MallocSpace;
use crate::util::alloc::Allocator;
use crate::util::conversions;
use crate::util::malloc::calloc;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::{plan::global::Plan, util::heap::layout::vm_layout_constants::PAGES_IN_CHUNK};

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
        trace!("alloc");
        assert!(offset == 0);
        assert!(align <= 16);
        self.space.unwrap().alloc(size)
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
