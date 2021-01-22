use crate::{policy::malloc::{HEAP_SIZE, heap_full, map_meta_space_for_chunk, meta_space_mapped, set_alloc_bit}, util::{conversions}};
use crate::policy::malloc::calloc;
use crate::policy::malloc::malloc_usable_size;
use crate::policy::malloc::HEAP_USED;
use crate::util::alloc::Allocator;
use crate::plan::global::Plan;
use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::OpaquePointer;
use crate::util::Address;
use crate::vm::VMBinding;
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
        assert!(offset==0);
        // println!("{} {}",_align, size);
        unsafe {
            if heap_full() {
                self.plan.handle_user_collection_request(self.tls, true);
                assert!(!heap_full(), "FreeListAllocator: Out of memory!");
            }
            let ptr = calloc(1, size);
            let address = Address::from_mut_ptr(ptr);
            if !meta_space_mapped(address) {
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
            }
            let allocated_memory = malloc_usable_size(ptr);
            set_alloc_bit(address);
            HEAP_USED.fetch_add(allocated_memory, Ordering::SeqCst);
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
        FreeListAllocator {
            tls,
            space,
            plan,
        }
    }
}