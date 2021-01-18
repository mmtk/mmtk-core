// a collection of functions and data structures used by MallocMS
// currently under policy so that is_malloced can be accessed by the OpenJDK binding
// once the sparse SFT table is in use and is_malloced is replaced by is_mapped_address, this should be moved to plan::mallocms

use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;
use crate::util::conversions;

use atomic::Ordering;
// Import calloc, free, and malloc_usable_size from the library specified in Cargo.toml:45
#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{free, malloc_usable_size, calloc};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{mi_free as free, mi_malloc_usable_size as malloc_usable_size, mi_calloc as calloc};

#[cfg(feature = "malloc_tcmalloc")]
pub use tcmalloc_sys::{TCMallocInternalCalloc as calloc, TCMallocInternalMallocSize as malloc_usable_size, TCMallocInternalFree as free};


// export LD_LIBRARY_PATH=/home/paiger/mmtk-openjdk/mmtk/target/release/build/hoard-sys-bcc6c3e0a7e92343/out/Hoard/src
#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{free, malloc_usable_size, calloc};

// export LD_LIBRARY_PATH=/home/paiger/mmtk-openjdk/mmtk/target/release/build/scalloc-sys-56cc58b9b944d8e4/out/scalloc/out/Release/lib.target
#[cfg(feature = "malloc_scalloc")]
pub use scalloc_sys::{free, malloc_usable_size, calloc};

#[cfg(not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc", feature = "malloc_tcmalloc", feature = "malloc_hoard", feature = "malloc_scalloc")))]
pub use libc::{free, malloc_usable_size, calloc};

lazy_static! {
    pub static ref METADATA_TABLE: RwLock<Vec<(usize, Vec<u8>, Vec<u8>)>> = RwLock::default();
    pub static ref METADATA_BUFFER: Mutex<Vec<Address>> = Mutex::default();
}

pub static mut HEAP_SIZE: usize = 0;
pub static HEAP_USED: AtomicUsize = AtomicUsize::new(0);

// Set the corresponding bit for each address in the buffer
pub fn write_metadata_bits() {
    let mut buffer = METADATA_BUFFER.lock().unwrap();
    let ref mut metadata_table = METADATA_TABLE.write().unwrap();
    loop {
        let address = match buffer.pop() {
            Some(addr) => addr,
            None => {    
                // buffer exhausted
                return
            },
        };
        let chunk_index = address_to_chunk_index_with_write(address, metadata_table);
        let chunk_index = match chunk_index {
            Some(i) => i,
            None => {
                let table_length = metadata_table.len();
                let malloced = vec![0; 1 << LOG_BYTES_IN_CHUNK >> 4];
                let marked = vec![0; 1 << LOG_BYTES_IN_CHUNK >> 4];
                let row = (conversions::chunk_align_down(address).as_usize(), malloced, marked);
                metadata_table.push(row);
                table_length
            }
        };
        let word_index = address_to_word_index(address);
        let row = &mut metadata_table[chunk_index];
        row.1[word_index] = 1;
    }
}

pub fn create_metadata(address: Address) {
    let buffer_full = {
        let mut buffer = METADATA_BUFFER.lock().unwrap();
        buffer.push(address);
        buffer.len() >= 16
    };
    if buffer_full {
        write_metadata_bits();
    }

}

pub fn heap_full() -> bool {
    unsafe { HEAP_USED.load(Ordering::SeqCst) >= HEAP_SIZE }
}

// Check the bit for a given object
// Later, this should be updated to use the SFT table defined in policy::space
pub fn is_malloced(object: ObjectReference) -> bool {
    if !METADATA_BUFFER.lock().unwrap().is_empty() {
        write_metadata_bits();
    }
    let chunk_index = {
        let ref metadata_table = METADATA_TABLE.read().unwrap();
        address_to_chunk_index_with_read(object.to_address(), metadata_table)
    };
    match chunk_index {
        Some(index) => {
            METADATA_TABLE.read().unwrap()[index].1[address_to_word_index(object.to_address())] == 1

        },
        None => {
            false
        },
    }

}

// There is an entry for each word
pub fn address_to_word_index(address: Address) -> usize {
    (address - conversions::chunk_align_down(address)) / 16
}

pub fn word_index_to_address(index: usize, chunk_start: usize) -> Address {
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
    let word_index = address_to_word_index(address);

    let ref metadata_table = METADATA_TABLE.read().unwrap();
    let chunk_index = match address_to_chunk_index_with_read(address, metadata_table) {
        Some(index) => index,
        None => unreachable!(), // this function should only be called on an object that is known to have been allocated by malloc
    };
    let row = &metadata_table[chunk_index];
    row.2[word_index] == 1
    
}