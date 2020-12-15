use std::sync::Mutex;
use std::collections::HashSet;
use bit_vec::BitVec;
//use roaring::bitmap::RoaringBitmap;
use crate::util::{Address, ObjectReference};

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    // pub static ref MALLOCED: Mutex<BitVec> = Mutex::default();
    // pub static ref MARKED: Mutex<BitVec> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
    pub static ref INITIAL: Mutex<usize> = Mutex::default();
}
pub const MALLOC_MEMORY: usize = 300000000;

pub unsafe fn malloc_memory_full() -> bool {
    *MEMORY_ALLOCATED.lock().unwrap() >= MALLOC_MEMORY
}

pub fn is_malloced(object: ObjectReference) -> bool {
    //using bitmaps
    // let malloced = MALLOCED.lock().unwrap().get(object.to_address().as_usize());
    // match malloced {
    //     None => false,
    //     Some(m) => m,
    // }

    //using hashset
    NODES.lock().unwrap().contains(&object)
}

pub fn object_reference_to_index(object: ObjectReference) -> usize {
    object.to_address().as_usize() / 16
}

pub fn index_to_object_reference(index: usize) -> ObjectReference {
    unsafe { Address::from_usize(index * 16).to_object_reference() }
}