use std::sync::Mutex;
use std::collections::HashSet;
use std::collections::HashMap;
use bit_vec::BitVec;
use crate::util::ObjectReference;
// use crate::policy::space::Space;
// use super::space::{CommonSpace, SFT};

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    //pub static ref NODES: Mutex<BitVec> = Mutex::default();
    // pub static ref MEMORY_MAP: Mutex<HashMap<ObjectReference, usize>> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
}
pub const MALLOC_MEMORY: usize = 1000000000;

pub unsafe fn malloc_memory_full() -> bool {
    *MEMORY_ALLOCATED.lock().unwrap() >= MALLOC_MEMORY
}


// lazy_static! {
//     pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
// }
// pub const MALLOC_MEMORY: usize = 1000000000;
// // pub static mut MEMORY_ALLOCATED: usize = 0;

// pub struct MallocSpace {
//     pub memory_allocated: usize,
//     //MALLOC_MEMORY: usize,
// }

// impl SFT for MallocSpace {
//     fn name(&self) -> &str {
//         unreachable!()
//     }
//     fn is_live(&self, object: ObjectReference) -> bool {
//         unreachable!()
//     }
//     fn is_movable(&self) -> bool {
//         unreachable!()
//     }
//     #[cfg(feature = "sanity")]
//     fn is_sane(&self) -> bool {
//         unreachable!()
//     }
//     fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {unreachable!()}
// }

// impl<VM: VMBinding> Space<VM> for MallocSpace {
//     fn as_space(&self) -> &dyn Space<VM> {
//         self
//     }
//     fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
//         self
//     }
//     fn get_page_resource(&self) -> &dyn PageResource<VM> {
//         unreachable!();
//     }
//     fn common(&self) -> &CommonSpace<VM> {
//         unreachable!();
//     }
//     unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
//         unreachable!();
//     }

//     fn init(&mut self, _vm_map: &'static VMMap) {
//         unreachable!();
//     }

//     fn release_multiple_pages(&mut self, _start: Address) {
//         unreachable!();
//     }
// }
// impl MallocSpace {
//     pub unsafe fn malloc_memory_full(&self) -> bool {
//         self.memory_allocated >= MALLOC_MEMORY
//     }

//     pub fn new() -> Self {
//         MallocSpace {
//             memory_allocated: 0,
//         }
//     }
// }
