use atomic::Ordering;

use crate::util::constants;
#[cfg(debug_assertions)]
use crate::util::constants::BYTES_IN_WORD;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
#[cfg(debug_assertions)]
use crate::util::metadata::address_to_meta_address;
use crate::util::metadata::load_atomic;
use crate::util::metadata::store_atomic;
#[cfg(target_pointer_width = "64")]
use crate::util::metadata::LOCAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::metadata::{MetadataSpec, SideMetadata};
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::{ObjectModel, VMBinding};

use std::collections::HashSet;
use std::sync::RwLock;

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

#[cfg(target_pointer_width = "32")]
pub(super) const ALLOC_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "64")]
pub(super) const ALLOC_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    ACTIVE_CHUNKS.read().unwrap().contains(&chunk_start)
}

pub fn map_meta_space_for_chunk(metadata: &SideMetadata, chunk_start: Address) {
    let mut active_chunks = ACTIVE_CHUNKS.write().unwrap();
    if active_chunks.contains(&chunk_start) {
        return;
    }
    active_chunks.insert(chunk_start);
    let mmap_metadata_result = metadata.try_map_metadata_space(chunk_start, BYTES_IN_CHUNK);
    debug_assert!(
        mmap_metadata_result.is_ok(),
        "mmap sidemetadata failed for chunk_start ({})",
        chunk_start
    );
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
        let ret = load_atomic(ALLOC_METADATA_SPEC, address, Ordering::SeqCst) == 1;
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

    load_atomic(ALLOC_METADATA_SPEC, address, Ordering::SeqCst) == 1
}

pub fn is_marked<VM: VMBinding>(object: ObjectReference) -> bool {
    // #[cfg(debug_assertions)]
    // if ASSERT_METADATA {
    //     // Need to make sure we atomically access the side metadata and the map.
    //     let lock = MARK_MAP.read().unwrap();
    //     // let ret = load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1;
    //     let ret = VM::VMObjectModel::get_mark_bit(object, Some(Ordering::SeqCst)) == 1;
    //     debug_assert_eq!(
    //         lock.contains(&unsafe { object.to_address().align_down(BYTES_IN_WORD).to_object_reference() }),
    //         ret,
    //         "is_marked(): mark bit does not match mark map, address = {} (aligned to {}), meta address = {}",
    //         object.to_address(),
    //         object.to_address().align_down(BYTES_IN_WORD),
    //         address_to_meta_address(MARKING_METADATA_SPEC, object.to_address())
    //     );
    //     return ret;
    // }

    // load_atomic(MARKING_METADATA_SPEC, object.to_address()) == 1
    VM::VMObjectModel::load_metadata(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        None,
        Some(Ordering::SeqCst),
    ) == 1
}

pub fn set_alloc_bit(object: ObjectReference) {
    // #[cfg(debug_assertions)]
    // if ASSERT_METADATA {
    //     // Need to make sure we atomically access the side metadata and the map.
    //     let mut lock = ALLOC_MAP.write().unwrap();
    //     store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 1, Ordering::SeqCst);
    //     lock.insert(object);
    //     return;
    // }

    store_atomic(
        ALLOC_METADATA_SPEC,
        object.to_address(),
        1,
        Ordering::SeqCst,
    );
}

pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference) {
    // #[cfg(debug_assertions)]
    // if ASSERT_METADATA {
    //     // Need to make sure we atomically access the side metadata and the map.
    //     let mut lock = MARK_MAP.write().unwrap();
    //     store_atomic(MARKING_METADATA_SPEC, object.to_address(), 1);
    //     lock.insert(object);
    //     return;
    // }

    // store_atomic(MARKING_METADATA_SPEC, object.to_address(), 1);
    VM::VMObjectModel::store_metadata(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        1,
        None,
        Some(Ordering::SeqCst),
    );
}

pub fn unset_alloc_bit(object: ObjectReference) {
    // #[cfg(debug_assertions)]
    // if ASSERT_METADATA {
    //     // Need to make sure we atomically access the side metadata and the map.
    //     let mut lock = ALLOC_MAP.write().unwrap();
    //     store_atomic(ALLOC_METADATA_SPEC, object.to_address(), 0, Ordering::SeqCst);
    //     lock.remove(&object);
    //     return;
    // }

    store_atomic(
        ALLOC_METADATA_SPEC,
        object.to_address(),
        0,
        Ordering::SeqCst,
    );
}

pub fn unset_mark_bit<VM: VMBinding>(object: ObjectReference) {
    // #[cfg(debug_assertions)]
    // if ASSERT_METADATA {
    //     // Need to make sure we atomically access the side metadata and the map.
    //     let mut lock = MARK_MAP.write().unwrap();
    //     store_atomic(MARKING_METADATA_SPEC, object.to_address(), 0, Ordering::SeqCst);
    //     lock.remove(&object);
    //     return;
    // }

    VM::VMObjectModel::store_metadata(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        0,
        None,
        Some(Ordering::SeqCst),
    );
}
