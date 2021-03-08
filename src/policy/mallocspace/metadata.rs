use crate::util::constants;
#[cfg(debug_assertions)]
use crate::util::constants::BYTES_IN_WORD;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::side_metadata::load_atomic;
use crate::util::side_metadata::meta_bytes_per_chunk;
use crate::util::side_metadata::store_atomic;
use crate::util::side_metadata::try_map_metadata_space;
use crate::util::side_metadata::SideMetadataScope;
use crate::util::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;

use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    pub static ref ACTIVE_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();
}

// We use the following hashset to assert if bits are set/unset properly in side metadata.
#[cfg(debug_assertions)]
const ASSERT_METADATA: bool = true;

#[cfg(debug_assertions)]
lazy_static! {
    pub static ref ALLOC_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
    pub static ref MARK_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
}

pub(super) const ALLOC_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

pub(super) const MARKING_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: ALLOC_METADATA_SPEC.offset
        + meta_bytes_per_chunk(
            ALLOC_METADATA_SPEC.log_min_obj_size,
            ALLOC_METADATA_SPEC.log_num_of_bits,
        ),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
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
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let lock = ALLOC_MAP.read().unwrap();
        let check =
            lock.contains(&unsafe { address.align_down(BYTES_IN_WORD).to_object_reference() });
        let ret = load_atomic(ALLOC_METADATA_SPEC, address) == 1;
        debug_assert_eq!(
            check,
            ret,
            "is_alloced_object(): alloc bit does not match alloc map, address = {} (aligned to {}), meta_start = {}",
            address,
            address.align_down(BYTES_IN_WORD),
            ALLOC_METADATA_SPEC.meta_start(address)
        );
        return ret;
    }

    load_atomic(ALLOC_METADATA_SPEC, address) == 1
}

pub fn is_marked(object: ObjectReference) -> bool {
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let lock = MARK_MAP.read().unwrap();
        let ret = load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1;
        debug_assert_eq!(
            lock.contains(&unsafe { object.to_address().align_down(BYTES_IN_WORD).to_object_reference() }),
            ret,
            "is_marked(): mark bit does not match mark map, address = {} (aligned to {}), meta_start = {}",
            object.to_address(),
            object.to_address().align_down(BYTES_IN_WORD),
            MARKING_METADATA_SPEC.meta_start(object.to_address())
        );
        return ret;
    }

    load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1
}

pub fn set_alloc_bit(object: ObjectReference) {
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let mut lock = ALLOC_MAP.write().unwrap();
        store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 1);
        lock.insert(object);
        return;
    }

    store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 1);
}

pub fn set_mark_bit(object: ObjectReference) {
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let mut lock = MARK_MAP.write().unwrap();
        store_atomic(MARKING_METADATA_SPEC, object.to_address(), 1);
        lock.insert(object);
        return;
    }

    store_atomic(MARKING_METADATA_SPEC, object.to_address(), 1);
}

pub fn unset_alloc_bit(object: ObjectReference) {
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let mut lock = ALLOC_MAP.write().unwrap();
        store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 0);
        lock.remove(&object);
        return;
    }

    store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 0);
}

pub fn unset_mark_bit(object: ObjectReference) {
    #[cfg(debug_assertions)]
    if ASSERT_METADATA {
        // Need to make sure we atomically access the side metadata and the map.
        let mut lock = MARK_MAP.write().unwrap();
        store_atomic(MARKING_METADATA_SPEC, object.to_address(), 0);
        lock.remove(&object);
        return;
    }

    store_atomic(MARKING_METADATA_SPEC, object.to_address(), 0);
}
