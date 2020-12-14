use std::sync::Mutex;
use std::collections::HashSet;
//use bit_vec::BitVec;
use crate::util::ObjectReference;

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    //pub static ref NODES: Mutex<BitVec> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
}
pub const MALLOC_MEMORY: usize = 1000000000;

pub unsafe fn malloc_memory_full() -> bool {
    *MEMORY_ALLOCATED.lock().unwrap() >= MALLOC_MEMORY
}

