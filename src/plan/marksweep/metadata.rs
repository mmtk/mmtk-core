use crate::util::{constants, conversions, side_metadata::{bzero_metadata_for_chunk, load_atomic, meta_bytes_per_chunk, store_atomic, try_mmap_metadata_chunk}};
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::side_metadata::SideMetadataSpec;
use crate::util::side_metadata::SideMetadataScope;
use crate::util::Address;
use crate::util::ObjectReference;
use conversions::chunk_align_down;
use std::{collections::HashSet, sync::Mutex};
use std::sync::RwLock;

lazy_static! {
    pub static ref ACTIVE_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();
    pub static ref NODES: Mutex<HashSet<usize>> = Mutex::default();
}

pub const ALLOC_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec { 
    scope: SideMetadataScope::Global, 
    offset: meta_bytes_per_chunk(constants::LOG_BYTES_IN_WORD as usize, 1), 
    log_num_of_bits: 0, 
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize };
pub const MARKING_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec { 
    scope: SideMetadataScope::Global, 
    offset: ALLOC_METADATA_SPEC.offset + meta_bytes_per_chunk(constants::LOG_BYTES_IN_WORD as usize, 0), 
    log_num_of_bits: 0, 
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize };

pub fn meta_space_mapped(address: Address) -> bool {
    let chunk_start = chunk_align_down(address);
    ACTIVE_CHUNKS.read().unwrap().contains(&chunk_start)
}

pub unsafe fn map_meta_space_for_chunk(chunk_start: Address) {
    // debug!("mapping meta space for chunk starting at address {}", chunk_start);
    try_mmap_metadata_chunk(chunk_start, BYTES_IN_CHUNK / 4, 0);
    bzero_metadata_for_chunk(ALLOC_METADATA_SPEC, chunk_start);
    bzero_metadata_for_chunk(MARKING_METADATA_SPEC, chunk_start);
    let mut a = chunk_start;
    while a - chunk_start < BYTES_IN_CHUNK {
        assert!(load_atomic(ALLOC_METADATA_SPEC, a) == 0);
        a = a.add(8);
    }
    ACTIVE_CHUNKS.write().unwrap().insert(chunk_start);
}

// Check if a given object was allocated by malloc
pub fn is_malloced(object: ObjectReference) -> bool {
    let address = object.to_address();
    let r = unsafe {
        meta_space_mapped(address)
            && load_atomic(ALLOC_METADATA_SPEC, address) == 1
    };

    assert!(r == NODES.lock().unwrap().contains(&address.as_usize()), "metadata gives {} but nodes gives {} for address {}", r, !r, address);
    r
}

// check if a given object is marked
pub fn is_marked(object: ObjectReference) -> bool {
    let address = object.to_address();
    debug_assert!(meta_space_mapped(address));
    unsafe { load_atomic(MARKING_METADATA_SPEC, address) == 1 } 
}

pub fn set_alloc_bit(address: Address) {
    debug!("set_alloc_bit at {}", address);
    debug_assert!(meta_space_mapped(address));
    unsafe {
        store_atomic(ALLOC_METADATA_SPEC, address, 1);
    }
}

pub fn set_mark_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        store_atomic(MARKING_METADATA_SPEC, address, 1);
    }
}

pub fn unset_alloc_bit(address: Address) {
    // debug!("unset_alloc_bit at {}", address);
    debug_assert!(meta_space_mapped(address));
    unsafe {
        store_atomic(ALLOC_METADATA_SPEC, address, 0);
    }
}

pub fn unset_mark_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        store_atomic(MARKING_METADATA_SPEC, address, 0);
    }
}
