use std::sync::{Mutex, atomic::AtomicUsize};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashSet;
use atomic::Ordering;

use crate::util::{queue::SharedQueue, raw_memory_freelist::RawMemoryFreeList};
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::conversions;
use std::collections::LinkedList;
use std::sync::atomic::AtomicU8;
// use buffer::Buffer;

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref MARKED: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref MEMORY_ALLOCATED: Mutex<usize> = Mutex::default();
    pub static ref METADATA_TABLE: RwLock<Vec<Option<(AtomicUsize, Vec<AtomicU8>, Vec<AtomicU8>)>>> = RwLock::default();
    pub static ref MALLOC_BUFFER: Mutex<Vec<(Address, u8)>> = Mutex::default();
    pub static ref MARK_BUFFER: Mutex<Vec<(Address, u8)>> = Mutex::default();
    
}
pub const MALLOC_MEMORY: usize = 90000000;
pub const USE_HASHSET: bool = false;
pub static mut PHASE: Phase = Phase::Allocation;
pub static mut COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn write_malloc_bits() {
    // unsafe { COUNT.fetch_add(1, Ordering::SeqCst);
    // println!("{}", COUNT.load(Ordering::SeqCst));
    // if COUNT.load(Ordering::SeqCst) > 10000 {
    //     println!("{}", MALLOC_BUFFER.lock().unwrap().len());
    // }
// }
    println!("called write_malloc_bits()");
    let mut local_buffer = {
        let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
        let mut local_buffer = vec![];
        for i in 0..malloc_buffer.len() {
            local_buffer.push(malloc_buffer.pop().unwrap());
        }
        local_buffer
    };
    println!("got our buffer");

    // println!("malloc_buffer.len() = {}", malloc_buffer.len());
    // if unsafe { PHASE == Phase::Sweeping } {
    //     // println!("locked table");
    // }
    loop {
        println!("begin loop");
        let (address, bit) = match local_buffer.pop() {
            Some(tuple) => tuple,
            None => {    
                // buffer exhausted
                // println!("completed writing malloc bits");
                return
            },
        };
        let ref mut metadata_table = METADATA_TABLE.write().unwrap();
        println!("write-locked table");
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                println!("need new row for chunk start {}, currently {}", conversions::chunk_align_down(address).as_usize(),table_length);
                // metadata_table.resize_with(table_length + 10, || None);
                let malloced = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                let marked = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                let row = Some((AtomicUsize::new(conversions::chunk_align_down(address).as_usize()), malloced, marked));
                metadata_table.push(row);
                println!("created new row");
                table_length
            }
        };
        let bitmap_index = address_to_bitmap_index(address);
        let mut row = metadata_table[chunk_index].as_mut().unwrap();
        row.1[bitmap_index] = AtomicU8::new(bit);
        println!("written to table");
    }
}

pub fn write_malloc_bits_with_write(metadata_table: &mut RwLockWriteGuard<Vec<Option<(AtomicUsize, Vec<AtomicU8>, Vec<AtomicU8>)>>>) {
    if unsafe { PHASE == Phase::Sweeping } {
        // println!("writing malloc bits");
    }
    // println!("We're writing malloc bits!");
    let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
    // println!("malloc_buffer.len() = {}", malloc_buffer.len());
    if unsafe { PHASE == Phase::Sweeping } {
        // println!("locked table");
    }
    loop {
        if unsafe { PHASE == Phase::Sweeping } {
            // println!("loop");
        }
        let address = malloc_buffer.pop();
        let mut address = match address {
            Some(address) => address,
            None => {    
                // println!("completed writing malloc bits");
                return
            },
        };
        // let ref mut metadata_table = METADATA_TABLE.write().unwrap();
        let (address, bit) = address;
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                metadata_table.resize_with(table_length + 1, || None);
                let malloced = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                let marked = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                metadata_table[table_length] = Some((AtomicUsize::new(conversions::chunk_align_down(address).as_usize()), malloced, marked));
                table_length
            }
        };
        let bitmap_index = address_to_bitmap_index(address);
        let mut row = metadata_table[chunk_index].as_mut().unwrap();
        row.1[bitmap_index] = AtomicU8::new(bit);
    }
}

pub fn write_mark_bits() {
    let mut mark_buffer = MARK_BUFFER.lock().unwrap();
    loop {
        let address = mark_buffer.pop();
        let mut address = match address {
            Some(address) => address,
            None => {    
                // println!("completed writing mark bits");
                return
            },
        };
        let ref mut metadata_table = METADATA_TABLE.write().unwrap();
        let (address, bit) = address;
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                metadata_table.resize_with(table_length + 1, || None);
                let malloced = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                let marked = (0..BYTES_IN_CHUNK).map(|_| AtomicU8::new(0)).collect();
                metadata_table[table_length] = Some((AtomicUsize::new(conversions::chunk_align_down(address).as_usize()), malloced, marked));
                table_length
            }
        };
        let bitmap_index = address_to_bitmap_index(address);
        let mut row = metadata_table[chunk_index].as_mut().unwrap();
        row.2[bitmap_index] = AtomicU8::new(bit);
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
    // need to update to not use hashset & account for buffering
    NODES.lock().unwrap().contains(&object)
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
    // unreachable!();
    let address = object.to_address();
    let bitmap_index = address_to_bitmap_index(address);
    if !MALLOC_BUFFER.lock().unwrap().is_empty() {
        write_malloc_bits();
    }
    if !MARK_BUFFER.lock().unwrap().is_empty() {
        write_mark_bits();
    }
    
    // let malloced = {
    //     let ref metadata_table = METADATA_TABLE.read().unwrap();
    //     let chunk_index = address_to_chunk_index_with_read(object.to_address(), metadata_table);
    //     match chunk_index {
    //         None => false,
    //         Some(index) => {
    //             let row = metadata_table[index].as_ref().unwrap();
    //             let bitmap_index = address_to_bitmap_index(object.to_address());
    //             row.1[bitmap_index].load(Ordering::SeqCst) == 1
    //         }
    //     }
    // };

    // println!("is object in NODES? {}. Is object malloced according to table? {}", NODES.lock().unwrap().contains(&object), malloced);
    let ref metadata_table = METADATA_TABLE.read().unwrap();
    let chunk_index = match address_to_chunk_index_with_read(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(), // this function should only be called on an object that is known to have been allocated by malloc
    };
    let row = metadata_table[chunk_index].as_ref().unwrap();
    row.2[bitmap_index].load(Ordering::SeqCst) == 1// FIXME needs to be updated to account for buffering
}


pub fn set_mark_bit(object: ObjectReference) {
    let address = object.to_address();
    let bitmap_index = address_to_bitmap_index(address);
    let chunk_start = conversions::chunk_align_down(address).as_usize();
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    let chunk_index = match address_to_chunk_index_with_write(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(),
    };
    let mut row = metadata_table[chunk_index].as_mut().unwrap();
    let ref mut marked = row.2;
    marked[address_to_bitmap_index(address)] = AtomicU8::new(1);
}

pub fn set_malloc_bit(object: ObjectReference) {
    let address = object.to_address();
    let bitmap_index = address_to_bitmap_index(address);
    let chunk_start = conversions::chunk_align_down(address).as_usize();
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    let chunk_index = match address_to_chunk_index_with_write(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(),
    };
    let mut row = metadata_table[chunk_index].as_mut().unwrap();
    let ref mut malloced = row.1;
    malloced[address_to_bitmap_index(address)] = AtomicU8::new(1);
}