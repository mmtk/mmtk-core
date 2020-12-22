
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
        // println!("allocing, malloc buffer size = {}", MALLOC_BUFFER.lock().unwrap().len());
        trace!("alloc");
        assert!(offset==0);
        unsafe {
            if malloc_memory_full() {
                PHASE = Phase::Marking;
                println!("collection time!");
                self.plan.handle_user_collection_request(self.tls, true);
                println!("collection done");
                assert!(!malloc_memory_full(), "FreeListAllocator: Out of memory!");
                PHASE = Phase::Allocation;
            }

            if USE_HASHSET {
                //using hashset
                let ptr = libc::calloc(1, size + 8);
                let address = Address::from_mut_ptr(ptr);
                let object = address.to_object_reference();
                let allocated_memory = libc::malloc_usable_size(ptr);
                let mut total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
                *total_memory_allocated += allocated_memory;
                // println!("Allocated {} bytes, total {} bytes.", allocated_memory, total_memory_allocated);
                // println!("allocated object sized {} into block sized {} at {:b}, index of {:b}", size + 8, allocated_memory, address.as_usize(), object_reference_to_index(object));
                NODES.lock().unwrap().insert(address.add(8).to_object_reference()); //NODES contains the reference to the object, not the mark word
                
                address.add(8)
            } else {
                //using bitmap
                let ptr = libc::calloc(1, size);
                let address = Address::from_mut_ptr(ptr);
                let object = address.to_object_reference();
                let allocated_memory = libc::malloc_usable_size(ptr);
                let mut total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
                *total_memory_allocated += allocated_memory;
                // add_to_list(address);
                NODES.lock().unwrap().insert(object);
                create_metadata(address);
                // println!("alloced {}", total_memory_allocated);
                address
            }
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