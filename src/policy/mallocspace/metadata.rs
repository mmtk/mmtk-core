use crate::util::constants;
#[cfg(debug_assertions)]
use crate::util::constants::BYTES_IN_WORD;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::side_metadata::address_to_meta_address;
use crate::util::side_metadata::load_atomic;
#[cfg(target_pointer_width = "32")]
use crate::util::side_metadata::meta_bytes_per_chunk;
use crate::util::side_metadata::store_atomic;
use crate::util::side_metadata::try_map_metadata_space;
use crate::util::side_metadata::SideMetadataScope;
use crate::util::side_metadata::SideMetadataSpec;
#[cfg(target_pointer_width = "64")]
use crate::util::side_metadata::{metadata_address_range_size, LOCAL_SIDE_METADATA_BASE_ADDRESS};
#[cfg(not(feature = "chunk_hashset"))]
use crate::util::side_metadata::GLOBAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::Address;
use crate::util::ObjectReference;

#[cfg(any(debug_assertions, feature = "chunk_hashset"))]
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

#[cfg(not(feature = "chunk_hashset"))]
static FIRST_CHUNK: AtomicBool = AtomicBool::new(true);

#[cfg(feature = "chunk_hashset")]
lazy_static! {
    pub static ref ACTIVE_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();
}

// We use the following hashset to assert if bits are set/unset properly in side metadata.
#[cfg(debug_assertions)]
const ASSERT_METADATA: bool = false;

#[cfg(debug_assertions)]
lazy_static! {
    pub static ref ALLOC_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
    pub static ref MARK_MAP: RwLock<HashSet<ObjectReference>> = RwLock::default();
}

#[cfg(all(target_pointer_width = "32", not(feature = "chunk_hashset")))]
pub(super) const ACTIVE_CHUNK_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::Global,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 0,
    log_min_obj_size: LOG_BYTES_IN_CHUNK as usize,
};

#[cfg(target_pointer_width = "32")]
pub(super) const ALLOC_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "32")]
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

#[cfg(all(target_pointer_width = "64", not(feature = "chunk_hashset")))]
pub(super) const ACTIVE_CHUNK_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::Global,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 0,
    log_min_obj_size: LOG_BYTES_IN_CHUNK as usize,
};

#[cfg(target_pointer_width = "64")]
pub(super) const ALLOC_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "64")]
pub(super) const MARKING_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: ALLOC_METADATA_SPEC.offset + metadata_address_range_size(ALLOC_METADATA_SPEC),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "64")]
pub(super) const ACTIVE_PAGE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::PolicySpecific,
    offset: MARKING_METADATA_SPEC.offset + metadata_address_range_size(MARKING_METADATA_SPEC),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_BYTES_IN_PAGE as usize,
};

// lazy_static! {
//     pub static ref MS_LOCAL_META_VEC: Vec<SideMetadataSpec> =
//         vec![ALLOC_METADATA_SPEC, MARKING_METADATA_SPEC, ACTIVE_PAGE_METADATA_SPEC];
// }

// #[cfg(not(feature = "chunk_hashset"))]
// lazy_static! {
//     pub static ref MS_ACTIVE_CHUNK_MAP: Vec<SideMetadataSpec> = vec![ACTIVE_CHUNK_METADATA_SPEC];
// }

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);

    #[cfg(feature = "chunk_hashset")]
    {
        ACTIVE_CHUNKS.read().unwrap().contains(&chunk_start)
    }

    #[cfg(not(feature = "chunk_hashset"))]
    {
        is_chunk_marked(chunk_start)
    }
}

#[cfg(not(feature = "chunk_hashset"))]
fn map_chunk_mark_space(chunk_start: Address) {
    if !try_map_metadata_space(
        chunk_start - 2048 * BYTES_IN_CHUNK, // start
        4096 * BYTES_IN_CHUNK,               // size
        &[ACTIVE_CHUNK_METADATA_SPEC],       // global metadata
        &[],                                 // local metadata
    ) {
        panic!("failed to mmap meta memory");
    }
    info!(
        "chunk_start = {} mapped space for {} -> {}",
        chunk_start,
        chunk_start - 2048 * BYTES_IN_CHUNK,
        chunk_start + 2048 * BYTES_IN_CHUNK
    );
}

pub fn map_meta_space_for_chunk(chunk_start: Address) {
    #[cfg(feature = "chunk_hashset")]
    {
        let mut active_chunks = ACTIVE_CHUNKS.write().unwrap();
        if active_chunks.contains(&chunk_start) {
            return;
        }
        active_chunks.insert(chunk_start);
        let mmap_metadata_result = try_map_metadata_space(
            chunk_start,
            BYTES_IN_CHUNK,
            &[],
            &[ALLOC_METADATA_SPEC, MARKING_METADATA_SPEC],
        );
        debug_assert!(
            mmap_metadata_result.is_ok(),
            "mmap sidemetadata failed for chunk_start ({})",
            chunk_start
        );
    }

    #[cfg(not(feature = "chunk_hashset"))]
    {
        if FIRST_CHUNK.load(Ordering::SeqCst) {
            map_chunk_mark_space(chunk_start);
            FIRST_CHUNK.store(false, Ordering::SeqCst);
        }

        if is_chunk_marked(chunk_start) {
            return;
        }

        set_chunk_mark_bit(chunk_start);
        let mmap_metadata_result = try_map_metadata_space(
            chunk_start,
            BYTES_IN_CHUNK,
            &[],
            &[ALLOC_METADATA_SPEC, MARKING_METADATA_SPEC],
        );
        trace!("set chunk mark bit for {}", chunk_start);
        debug_assert!(
            mmap_metadata_result.is_ok(),
            "mmap sidemetadata failed for chunk_start ({})",
            chunk_start
        );
    }
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
            "is_alloced_object(): alloc bit does not match alloc map, address = {} (aligned to {}), meta address = {}",
            address,
            address.align_down(BYTES_IN_WORD),
            address_to_meta_address(ALLOC_METADATA_SPEC, address)
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
            "is_marked(): mark bit does not match mark map, address = {} (aligned to {}), meta address = {}",
            object.to_address(),
            object.to_address().align_down(BYTES_IN_WORD),
            address_to_meta_address(MARKING_METADATA_SPEC, object.to_address())
        );
        return ret;
    }

    load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1
}

#[cfg(not(feature = "chunk_hashset"))]
pub fn is_chunk_marked(chunk_start: Address) -> bool {
    if FIRST_CHUNK.load(Ordering::SeqCst) {
        return false; // if first chunk has not been mapped, then no chunk is marked
    }

    load_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start) == 1
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

#[cfg(not(feature = "chunk_hashset"))]
pub fn set_chunk_mark_bit(chunk_start: Address) {
    store_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 1);
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

#[cfg(not(feature = "chunk_hashset"))]
pub fn unset_chunk_mark_bit(chunk_start: Address) {
    store_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 0);
}
