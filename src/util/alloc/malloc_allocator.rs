use crate::plan::global::Plan;
use crate::plan::marksweep::malloc::ms_calloc;
use crate::plan::marksweep::malloc::ms_malloc_usable_size;
use crate::plan::marksweep::metadata::map_meta_space_for_chunk;
use crate::plan::marksweep::metadata::meta_space_mapped;
use crate::plan::marksweep::metadata::set_alloc_bit;
use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::conversions;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use atomic::Ordering;
use std::sync::atomic::AtomicUsize;

pub static mut HEAP_SIZE: usize = 0;
pub static HEAP_USED: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    space: Option<&'static dyn Space<VM>>,
    plan: &'static SelectedPlan<VM>,
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
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, _align: usize, offset: isize) -> Address {
        trace!("alloc");
        debug_assert!(offset == 0);
        unsafe {
            let ptr = ms_calloc(1, size);
            let address = Address::from_mut_ptr(ptr);
            if !meta_space_mapped(address) {
                self.plan.poll(true, &self.plan.space);
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
            }
            set_alloc_bit(address);
            HEAP_USED.fetch_add(ms_malloc_usable_size(ptr), Ordering::SeqCst);
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
        plan: &'static SelectedPlan<VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
