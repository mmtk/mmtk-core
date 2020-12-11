

use crate::util::Address;
use crate::policy::malloc::*;
use crate::util::alloc::Allocator;
use crate::plan::global::Plan;
use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

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
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        //println!("called free list alloc");
        trace!("alloc");
        assert!(offset==0);
        unsafe {
            if malloc_memory_full() {
                self.plan.handle_user_collection_request(self.tls, true);
            }
            debug_assert!(!malloc_memory_full(), "FreeListAllocator: Out of memory!");
        }


        let ptr = unsafe { libc::calloc(1, size + 8) };
        let a = Address::from_mut_ptr(ptr);
        unsafe { a.store(0); }
        unsafe { MEMORY_ALLOCATED += libc::malloc_usable_size(a.to_mut_ptr()); }
        unsafe { NODES.lock().unwrap().insert(Address::from_usize(a.as_usize() + 8).to_object_reference()); } //a is the reference to the object, not the mark word
        
        a + 8usize

    }


    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        unimplemented!();
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