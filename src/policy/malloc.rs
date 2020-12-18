use std::sync::Mutex;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashSet;
use bit_vec::BitVec;
use crate::util::{Address, ObjectReference, heap::layout::vm_layout_constants::{BYTES_IN_CHUNK}};
use crate::util::conversions;
use std::sync::atomic::{AtomicU8, Ordering, AtomicUsize};

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
    pub static ref METADATA_TABLE: RwLock<Vec<Option<(AtomicUsize, Vec<AtomicU8>, Vec<AtomicU8>)>>> = RwLock::default();
}
pub const MALLOC_MEMORY: usize = 90000000;
pub const USE_HASHSET: bool = false;

pub unsafe fn malloc_memory_full() -> bool {
    *MEMORY_ALLOCATED.lock().unwrap() >= MALLOC_MEMORY
}

pub fn is_malloced(object: ObjectReference) -> bool {
    if USE_HASHSET {
        //using hashset
        NODES.lock().unwrap().contains(&object)
    } else {
        //using bitmaps
        let ref metadata_table = METADATA_TABLE.read().unwrap();
        let address = object.to_address();
        let chunk_index: usize = match address_to_chunk_index_with_read(address, metadata_table) {
            Some(index) => index,
            None => return false,
        };
        let row = metadata_table[chunk_index].as_ref().unwrap();
        let bytemap_index = address_to_bytemap_index(address);
        row.1[bytemap_index].load(Ordering::SeqCst) == 1
    }
}

pub fn create_metadata(address: Address) {
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    let chunk_index: usize = match address_to_chunk_index_with_write(address, metadata_table) {
        Some(index) => index,
        None => metadata_table.len(),
    };

    if chunk_index >= metadata_table.len() {
        let table_length = metadata_table.len();
        metadata_table.resize_with(table_length + 1, || None);
    }
    
    if metadata_table[chunk_index].is_none() {
        let chunk_start = AtomicUsize::new((conversions::chunk_align_down(address)).as_usize());
        metadata_table[chunk_index] = Some((chunk_start, (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect(), (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect()));
    }

    let bytemap_index = address_to_bytemap_index(address);

    let mut row = metadata_table[chunk_index].as_mut().unwrap();
    let ref mut malloced = row.1;
    malloced[bytemap_index] = AtomicU8::new(1);
}

pub fn address_to_bytemap_index(address: Address) -> usize {
    address - conversions::chunk_align_down(address)
}

pub fn bytemap_index_to_address(index: usize, chunk_start: usize) -> Address {
    unsafe { Address::from_usize(index).add(chunk_start) }
}

pub fn address_to_chunk_index_with_write(address: Address, metadata_table: &mut RwLockWriteGuard<Vec<Option<(AtomicUsize, Vec<AtomicU8>, Vec<AtomicU8>)>>>) -> Option<usize> {
    let chunk_start = conversions::chunk_align_down(address);
    let mut chunk_index = 0;
    while chunk_index < metadata_table.len() {
        let row = metadata_table[chunk_index].as_ref().unwrap();
        if row.0.load(Ordering::SeqCst) == chunk_start.as_usize() {
            return Some(chunk_index);
        }
        chunk_index += 1;
    }
    None
}

pub fn address_to_chunk_index_with_read(address: Address, metadata_table: &RwLockReadGuard<Vec<Option<(AtomicUsize, Vec<AtomicU8>, Vec<AtomicU8>)>>>) -> Option<usize> {
    let chunk_start = conversions::chunk_align_down(address);
    let mut chunk_index = 0;
    while chunk_index < metadata_table.len() {
        let row = metadata_table[chunk_index].as_ref().unwrap();
        if row.0.load(Ordering::SeqCst) == chunk_start.as_usize() {
            return Some(chunk_index);
        }
        chunk_index += 1;
    }
    None
}

pub fn is_marked(object: ObjectReference) -> bool {
    let ref metadata_table = METADATA_TABLE.read().unwrap();
    let address = object.to_address();
    let chunk_index = match address_to_chunk_index_with_read(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(), // this function should only be called on an object that is known to have been allocated by malloc
    };
    let row = metadata_table[chunk_index].as_ref().unwrap();
    let bytemap_index = address_to_bytemap_index(address);
    row.2[bytemap_index].load(Ordering::SeqCst) == 1
}


pub fn mark(object: ObjectReference) {
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    let address = object.to_address();
    let chunk_index = match address_to_chunk_index_with_write(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(),
    };
    let bytemap_index = address_to_bytemap_index(address);
    let chunk_start = AtomicUsize::new(conversions::chunk_align_down(address).as_usize());
    let mut row = metadata_table[chunk_index].as_mut().unwrap();
    let ref mut marked = row.2;
    marked[address_to_bytemap_index(address)] = AtomicU8::new(1);
}