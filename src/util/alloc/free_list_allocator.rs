use crate::{plan::global::Plan};
use crate::plan::selected_plan::SelectedPlan;
use crate::plan::mallocms::metadata::HEAP_USED;
use crate::plan::mallocms::metadata::heap_full;
use crate::plan::mallocms::metadata::map_meta_space_for_chunk;
use crate::plan::mallocms::metadata::meta_space_mapped;
use crate::plan::mallocms::metadata::set_alloc_bit;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::alloc::malloc::calloc;
use crate::util::alloc::malloc::malloc_usable_size;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::util::conversions;
use atomic::Ordering;

#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static dyn Space<VM>>,
    plan: &'static SelectedPlan<VM>,
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    pub fn rebind(&mut self, space: Option<&'static dyn Space<VM>>) {
        self.space = space;
    }
}

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
    }
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, _align: usize, offset: isize) -> Address {
        trace!("alloc");
        assert!(offset == 0);
        if heap_full() {
            self.plan.handle_user_collection_request(self.tls, true);
            assert!(!heap_full(), "FreeListAllocator: Out of memory!");
        }
        unsafe {
            let ptr = calloc(1, size);
            let address = Address::from_mut_ptr(ptr);
            if !meta_space_mapped(address) {
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
            }
            set_alloc_bit(address);
            HEAP_USED.fetch_add(malloc_usable_size(ptr), Ordering::SeqCst);
            address
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, _size: usize, _align: usize, _offset: isize) -> Address {
        unreachable!(); // No fast path so unnecessary
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static SelectedPlan<VM>,
    ) -> Self {
        FreeListAllocator { tls, space, plan }
    }
}
