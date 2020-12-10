//use libc::malloc;

use std::convert::TryInto;

use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::Address;

use crate::util::alloc::Allocator;

use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

//const BYTES_IN_PAGE: usize = 1 << 12;
//const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
//const BLOCK_MASK: usize = BLOCK_SIZE - 1;

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

        trace!("alloc");
        //println!("alloc");
        assert!(offset==0);

        // #[link(name = "c")]
        // use std::ffi::c_void;
        // extern "C" {
        //     fn malloc(size: usize) -> *mut c_void;
        // }
        
        let ptr = unsafe { libc::calloc(1, size + 8) };
        let a = Address::from_mut_ptr(ptr);
        //println!("alloc'd to {}", a);
        a + 8usize
        //align_allocation_no_fill::<VM>(a, align, offset)

        // let ptr = unsafe { ptr as usize + ptr.align_offset(align) };
        // println!("result = {}", ptr);
        // println!("offset = {}", offset);
        // unsafe { Address::from_usize(ptr) }
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