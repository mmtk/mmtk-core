use atomic::Ordering;

use crate::util::constants;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::GLOBAL_SIDE_METADATA_BASE_OFFSET;
use crate::util::Address;
use crate::util::ObjectReference;

/// This is the metadata spec for the alloc-bit.
///
/// An alloc-bit is required per min-object-size aligned address , rather than per object, and can only exist as side metadata.
///
#[cfg(target_pointer_width = "32")]
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: GLOBAL_SIDE_METADATA_BASE_OFFSET,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "64")]
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: GLOBAL_SIDE_METADATA_BASE_OFFSET,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

pub fn map_meta_space_for_chunk(metadata: &SideMetadataContext, chunk_start: Address) {
    let mmap_metadata_result = metadata.try_map_metadata_space(chunk_start, BYTES_IN_CHUNK);
    debug_assert!(
        mmap_metadata_result.is_ok(),
        "mmap sidemetadata failed for chunk_start ({})",
        chunk_start
    );
}

pub fn set_alloc_bit(object: ObjectReference) {
    debug_assert!(!is_alloced(object), "{:x}: alloc bit already set", object);
    side_metadata::store_atomic(
        &ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        1,
        Ordering::SeqCst,
    );
}

pub fn unset_addr_alloc_bit(address: Address) {
    debug_assert!(
        is_alloced_object(address),
        "{:x}: alloc bit not set",
        address
    );
    side_metadata::store_atomic(&ALLOC_SIDE_METADATA_SPEC, address, 0, Ordering::SeqCst);
}

pub fn unset_alloc_bit(object: ObjectReference) {
    debug_assert!(is_alloced(object), "{:x}: alloc bit not set", object);
    side_metadata::store_atomic(
        &ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        0,
        Ordering::SeqCst,
    );
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
///
pub unsafe fn unset_alloc_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_alloced(object), "{:x}: alloc bit not set", object);
    side_metadata::store(&ALLOC_SIDE_METADATA_SPEC, object.to_address(), 0);
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

pub fn is_alloced_object(address: Address) -> bool {
    side_metadata::load_atomic(&ALLOC_SIDE_METADATA_SPEC, address, Ordering::SeqCst) == 1
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
///
pub unsafe fn is_alloced_object_unsafe(address: Address) -> bool {
    side_metadata::load(&ALLOC_SIDE_METADATA_SPEC, address) == 1
}

pub fn bzero_alloc_bit(start: Address, size: usize) {
    side_metadata::bzero_metadata(&ALLOC_SIDE_METADATA_SPEC, start, size);
}
