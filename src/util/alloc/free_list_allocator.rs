
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
        trace!("alloc");
        assert!(offset==0);
        unsafe {
            if malloc_memory_full() {
                self.plan.handle_user_collection_request(self.tls, true);
                assert!(!malloc_memory_full(), "FreeListAllocator: Out of memory!");
            }

            //using hashset
            let ptr = libc::calloc(1, size + 8);
            let address = Address::from_mut_ptr(ptr);
            let object = address.to_object_reference();
            let block_size = libc::malloc_usable_size(ptr);
            let mut mem = MEMORY_ALLOCATED.lock().unwrap();
            *mem += block_size;
            println!("allocated object sized {} into block sized {} at {:b}, index of {:b}", size + 8, block_size, address.as_usize(), object_reference_to_index(object));

            NODES.lock().unwrap().insert(address.add(8).to_object_reference()); //NODES contains the reference to the object, not the mark word
            address.add(8)
            // println!("allocated to address = {a}/{a:b}", a=address.as_usize());

            //using bitmap
            // let ptr = libc::calloc(1, size);
            // let address = Address::from_mut_ptr(ptr);
            // let object = address.to_object_reference();
            // let obj_size = libc::malloc_usable_size(ptr);
            // let mut mem = MEMORY_ALLOCATED.lock().unwrap();
            // *mem += obj_size;

            // let mut MALLOCED_mut = MALLOCED.lock().unwrap();
            // let mut MARKED_mut = MARKED.lock().unwrap();
            // let index = object_reference_to_index(object);
            // println!("index = {}", index);
            // let grow_by = index - MALLOCED_mut.capacity() + 1;
            // if grow_by >= 0 {
            //     MALLOCED_mut.grow(grow_by, false);
            //     MARKED_mut.grow(grow_by, false);
            // }
            // MALLOCED_mut.set(index, true);

            // address
        }
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