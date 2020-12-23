use std::sync::{Mutex, atomic::AtomicUsize};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashSet;
use atomic::Ordering;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::conversions;
use std::sync::atomic::AtomicU8;

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref MARKED: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
    pub static ref METADATA_TABLE: RwLock<Vec<(usize, Vec<u8>, Vec<u8>)>> = RwLock::default();
    pub static ref MALLOC_BUFFER: Mutex<Vec<(Address, u8)>> = Mutex::default();
    pub static ref MARK_BUFFER: Mutex<Vec<(Address, u8)>> = Mutex::default();
    
}
pub const MALLOC_MEMORY: usize = 90000000;
pub const USE_HASHSET: bool = false;
pub static mut PHASE: Phase = Phase::Allocation;
pub static mut COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn write_malloc_bits() {
    let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
    // println!("called write_malloc_bits()");
    // let mut local_buffer = {
    //     let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
    //     let mut local_buffer = vec![];
    //     for i in 0..malloc_buffer.len() {
    //         local_buffer.push(malloc_buffer.pop().unwrap());
    //     }
    //     local_buffer
    // };
    // println!("got our buffer");

    // println!("malloc_buffer.len() = {}", malloc_buffer.len());
    // if unsafe { PHASE == Phase::Sweeping } {
    //     // println!("locked table");
    // }
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    loop {
        // println!("begin loop");
        let (address, bit) = match malloc_buffer.pop() {
            Some(tuple) => tuple,
            None => {    
                // buffer exhausted
                // println!("completed writing malloc bits");
                return
            },
        };
        // println!("write-locked table");
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                // println!("need new row for chunk start {}, currently {}", conversions::chunk_align_down(address).as_usize(),table_length);
                let malloced = vec![0; BYTES_IN_CHUNK/16];
                let marked = vec![0; BYTES_IN_CHUNK/16];
                let row = (conversions::chunk_align_down(address).as_usize(), malloced, marked);
                metadata_table.push(row);
                // println!("created new row");
                table_length
            }
        };
        let bitmap_index = address_to_bitmap_index(address);
        // if bit == 1 {
        //     println!("marking address {}, chunk_index = {}, bitmap_index = {}", address, chunk_index, bitmap_index);
        // }
        let mut row = &mut metadata_table[chunk_index];
        row.1[bitmap_index] = bit;
        // println!("written to table");
    }
}

pub fn write_mark_bits() {
    let mut mark_buffer = MARK_BUFFER.lock().unwrap();
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    loop {
        let address = mark_buffer.pop();
        let mut address = match address {
            Some(address) => address,
            None => {    
                // println!("completed writing mark bits");
                return
            },
        };
        let (address, bit) = address;
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                let malloced = vec![0; BYTES_IN_CHUNK/16];
                let marked = vec![0; BYTES_IN_CHUNK/16];
                let row = (conversions::chunk_align_down(address).as_usize(), malloced, marked);
                metadata_table.push(row);
                table_length
            }
        };
        let bitmap_index = address_to_bitmap_index(address);
        let mut row = &mut metadata_table[chunk_index];
        // if bit == 1 {
        //     println!("marking address {}", address);
        // }
        row.2[bitmap_index] = bit;
    }
}

pub unsafe fn malloc_memory_full() -> bool {
    *MEMORY_ALLOCATED.lock().unwrap() >= MALLOC_MEMORY
}

pub fn create_metadata(address: Address) {
    // println!("on cree des metadonnes");
    let buffer_full = {
        let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
        malloc_buffer.push((address, 1));
        malloc_buffer.len() >= 16
    };
    if buffer_full {
        write_malloc_bits();
    }
    // println!("on a termine")

}

pub fn is_malloced(object: ObjectReference) -> bool {
    // let nodes_result = NODES.lock().unwrap().contains(&object);
    // println!("checking address {}", object.to_address());
    if !MALLOC_BUFFER.lock().unwrap().is_empty() {
        write_malloc_bits();
    }
    let chunk_index = {
        let ref metadata_table = METADATA_TABLE.read().unwrap();
        address_to_chunk_index_with_read(object.to_address(), metadata_table)
    };
    match chunk_index {
        Some(index) => {
            let r = METADATA_TABLE.read().unwrap()[index].1[address_to_bitmap_index(object.to_address())] == 1;
            // assert!(r == nodes_result, "for address {}, testing hashset gives {}, testing metadata_table gives {}", object.to_address(), nodes_result, r);
            r
        },
        None => {
            // assert!(nodes_result == false);
            false
        },
    }

}

#[derive(Debug)]
pub enum Phase {
    Allocation,
    Marking,
    Sweeping,
}

impl PartialEq for Phase {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Phase::Allocation => {
                match other {
                    Phase::Allocation => true,
                    _ => false,
                }
            }
            Phase::Marking => {
                match other {
                    Phase::Marking => true,
                    _ => false,
                }
            }
            Phase::Sweeping => {
                match other {
                    Phase::Sweeping => true,
                    _ => false,
                }
            }
        }
    }
}

pub fn address_to_bitmap_index(address: Address) -> usize {
    (address - conversions::chunk_align_down(address)) / 16
}

pub fn bitmap_index_to_address(index: usize, chunk_start: usize) -> Address {
    unsafe { Address::from_usize(index * 16 + chunk_start) }
}

pub fn address_to_chunk_index_with_write(address: Address, metadata_table: &mut RwLockWriteGuard<Vec<(usize, Vec<u8>, Vec<u8>)>>) -> Option<usize> {
    let chunk_start = conversions::chunk_align_down(address);
    let mut chunk_index = 0;
    while chunk_index < metadata_table.len() {
        let row = &metadata_table[chunk_index];
        if row.0 == chunk_start.as_usize() {
            return Some(chunk_index);
        }
        chunk_index += 1;
    }
    None
}

pub fn address_to_chunk_index_with_read(address: Address, metadata_table: &RwLockReadGuard<Vec<(usize, Vec<u8>, Vec<u8>)>>) -> Option<usize> {
    
    let chunk_start = conversions::chunk_align_down(address);
    let mut chunk_index = 0;
    while chunk_index < metadata_table.len() {
        let row = &metadata_table[chunk_index];
        if row.0 == chunk_start.as_usize() {
            return Some(chunk_index);
        }
        chunk_index += 1;
    }
    None
}

pub fn is_marked(object: ObjectReference) -> bool {
    let address = object.to_address();
    let bitmap_index = address_to_bitmap_index(address);
    //it's cheaper not to check if the buffers are empty, because we have to lock them to check anyway
    write_malloc_bits();
    write_mark_bits();
    // if !MALLOC_BUFFER.lock().unwrap().is_empty() {
    //     write_malloc_bits();
    // }
    // if !MARK_BUFFER.lock().unwrap().is_empty() {
    //     write_mark_bits();
    // }

    let ref metadata_table = METADATA_TABLE.read().unwrap();
    let chunk_index = match address_to_chunk_index_with_read(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(), // this function should only be called on an object that is known to have been allocated by malloc
    };
    let row = &metadata_table[chunk_index];
    row.2[bitmap_index] == 1
    
}