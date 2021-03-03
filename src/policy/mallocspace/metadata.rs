use crate::util::constants;
use crate::util::conversions;
use crate::util::side_metadata::load_atomic;
use crate::util::side_metadata::meta_bytes_per_chunk;
use crate::util::side_metadata::store_atomic;
use crate::util::side_metadata::try_map_metadata_space;
use crate::util::side_metadata::SideMetadataScope;
use crate::util::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;

use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    pub static ref ACTIVE_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();

    pub static ref ALLOC_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
    pub static ref MARK_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
}

const ALLOC_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
};

const MARKING_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: ALLOC_METADATA_SPEC.offset
        + meta_bytes_per_chunk(
            ALLOC_METADATA_SPEC.log_min_obj_size,
            ALLOC_METADATA_SPEC.log_num_of_bits,
        ),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
};

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    ACTIVE_CHUNKS.read().unwrap().contains(&chunk_start)
}

pub fn map_meta_space_for_chunk(chunk_start: Address) {
    let mut active_chunks = ACTIVE_CHUNKS.write().unwrap();
    if active_chunks.contains(&chunk_start) {
        return;
    }
    active_chunks.insert(chunk_start);
    let mmap_metadata_result = try_map_metadata_space(
        chunk_start,
        BYTES_IN_CHUNK,
        0,
        meta_bytes_per_chunk(
            ALLOC_METADATA_SPEC.log_min_obj_size,
            ALLOC_METADATA_SPEC.log_num_of_bits,
        ) + meta_bytes_per_chunk(
            MARKING_METADATA_SPEC.log_min_obj_size,
            MARKING_METADATA_SPEC.log_num_of_bits,
        ),
    );
    debug_assert!(mmap_metadata_result, "mmap sidemetadata failed");
}

// Check if a given object was allocated by malloc
pub fn is_alloced_by_malloc(object: ObjectReference) -> bool {
    is_meta_space_mapped(object.to_address()) && is_alloced(object)
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

pub fn is_alloced_object(address: Address) -> bool {
    let alloc_map = ALLOC_MAP.read().unwrap();
    let ret = load_atomic(ALLOC_METADATA_SPEC, address) == 1;
    debug_assert_eq!(alloc_map.contains(&unsafe { address.to_object_reference() }), ret, "is_alloced_object(): alloc bit does not match alloc map");
    ret
}

pub fn is_marked(object: ObjectReference) -> bool {
    let mark_map = MARK_MAP.read().unwrap();
    let ret = load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1;
    debug_assert_eq!(mark_map.contains(&object), ret, "is_marked(): mark bit does not match mark map");
    ret
}

pub fn set_alloc_bit(object: ObjectReference) {
    let mut alloc_map = ALLOC_MAP.write().unwrap();
    store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 1);
    alloc_map.insert(object);
}

pub fn set_mark_bit(object: ObjectReference) {
    let mut mark_map = MARK_MAP.write().unwrap();
    store_atomic(MARKING_METADATA_SPEC, object.to_address(), 1);
    mark_map.insert(object);
}

pub fn unset_alloc_bit(object: ObjectReference) {
    let mut alloc_map = ALLOC_MAP.write().unwrap();
    store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 0);
    alloc_map.remove(&object);
}

pub fn unset_mark_bit(object: ObjectReference) {
    let mut mark_map = MARK_MAP.write().unwrap();
    store_atomic(MARKING_METADATA_SPEC, object.to_address(), 0);
    mark_map.remove(&object);
}
