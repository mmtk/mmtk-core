// a collection of functions and data structures used by MarkSweep
// currently under policy so that is_malloced can be accessed by the OpenJDK binding
// once the sparse SFT table is in use and is_malloced is replaced by is_mapped_address, this should be moved to plan::marksweep

use std::sync::atomic::{Ordering, AtomicUsize};
use std::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashSet;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::conversions;

lazy_static! {
    pub static ref NODES: Mutex<HashSet<ObjectReference>> = Mutex::default();
    // pub static ref MARKED: Mutex<HashSet<ObjectReference>> = Mutex::default();
    pub static ref METADATA_TABLE: RwLock<Vec<(usize, Vec<u8>, Vec<u8>)>> = RwLock::default();
    pub static ref MALLOC_BUFFER: Mutex<Vec<Address>> = Mutex::default();
    // pub static ref MARK_BUFFER: Mutex<Vec<Address>> = Mutex::default();
    
}
pub const MALLOC_MEMORY: usize = 90000000;
pub const USE_HASHSET: bool = false;
pub static MEMORY_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

// Import calloc, free, and malloc_usable_size from the library specified in Cargo.toml:45

#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{free, malloc_usable_size, calloc};

use libc::{c_void, size_t};
#[cfg(feature = "malloc_mimalloc")]
use mimalloc_sys::{mi_free, mi_malloc_usable_size, mi_calloc};

#[cfg(feature = "malloc_mimalloc")]
pub unsafe fn malloc_usable_size(p: *const c_void) -> size_t {
    mi_malloc_usable_size(p)
}

#[cfg(feature = "malloc_mimalloc")]
pub unsafe fn free(p: *mut c_void) {
    mi_free(p);
}

#[cfg(feature = "malloc_mimalloc")]
pub unsafe fn calloc(count: size_t, size: size_t) -> *mut c_void {
    mi_calloc(count, size)
}

#[cfg(not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc")))]
pub use libc::{free, malloc_usable_size, calloc};

// Set the corresponding bit for each address in the buffer
pub fn write_malloc_bits() {
    let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    loop {
        // println!("begin loop");
        let address = match malloc_buffer.pop() {
            Some(addr) => addr,
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
        let mut row = &mut metadata_table[chunk_index];
        row.1[bitmap_index] = 1;
        // println!("written to table");
    }
}

pub unsafe fn malloc_memory_full() -> bool {
    MEMORY_ALLOCATED.load(Ordering::SeqCst) >= MALLOC_MEMORY
}

pub fn create_metadata(address: Address) {
    // println!("on cree des metadonnes");
    let buffer_full = {
        let mut malloc_buffer = MALLOC_BUFFER.lock().unwrap();
        malloc_buffer.push(address);
        malloc_buffer.len() >= 16
    };
    if buffer_full {
        write_malloc_bits();
    }
    // println!("on a termine")

}

// Check the bit for a given object
// Later, this should be updated to use the SFT table defined in policy::space
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

// There is an entry for each word
pub fn address_to_bitmap_index(address: Address) -> usize {
    (address - conversions::chunk_align_down(address)) / 16
}

pub fn bitmap_index_to_address(index: usize, chunk_start: usize) -> Address {
    unsafe { Address::from_usize(index * 16 + chunk_start) }
}

// find the index in the metadata table for the chunk into which an address fits
// use a metadata_table locked for writing
// is there a better way to do this?
pub fn address_to_chunk_index_with_write(address: Address, metadata_table: &mut RwLockWriteGuard<Vec<(usize, Vec<u8>, Vec<u8>)>>) -> Option<usize> {
    let chunk_start = conversions::chunk_align_down(address);
    metadata_table.iter().position(|row| row.0 == chunk_start.as_usize())
}

// use a metadata_table locked for reading
pub fn address_to_chunk_index_with_read(address: Address, metadata_table: &RwLockReadGuard<Vec<(usize, Vec<u8>, Vec<u8>)>>) -> Option<usize> {
    let chunk_start = conversions::chunk_align_down(address);
    metadata_table.iter().position(|row| row.0 == chunk_start.as_usize())
}

// check the corresponding bit in the metadata table
pub fn is_marked(object: ObjectReference) -> bool {
    let address = object.to_address();
    let bitmap_index = address_to_bitmap_index(address);

    let ref metadata_table = METADATA_TABLE.read().unwrap();
    let chunk_index = match address_to_chunk_index_with_read(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(), // this function should only be called on an object that is known to have been allocated by malloc
    };
    let row = &metadata_table[chunk_index];
    row.2[bitmap_index] == 1
    
}