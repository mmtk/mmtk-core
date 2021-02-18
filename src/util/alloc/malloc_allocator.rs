use crate::policy::mallocspace::metadata::{
    map_meta_space_for_chunk, meta_space_mapped, set_alloc_bit,
};
use crate::policy::space::Space;
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
    space: Option<&'static dyn Space<VM>>,
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn rebind(&mut self, space: Option<&'static dyn Space<VM>>) {
        self.space = space;
    }
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
    }
    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, _align: usize, offset: isize) -> Address {
        trace!("alloc");
        debug_assert!(offset == 0);
        unsafe {
            let ptr = calloc(1, size);
            let address = Address::from_mut_ptr(ptr);
            if !meta_space_mapped(address) {
                self.plan.poll(false, self.space.unwrap());
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
                self.space
                    .unwrap()
                    .get_page_resource()
                    .reserve_pages(PAGES_IN_CHUNK);
            }
            set_alloc_bit(address);
            address
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, _size: usize, _align: usize, _offset: isize) -> Address {
        unreachable!();
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
