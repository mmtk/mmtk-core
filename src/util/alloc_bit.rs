use atomic::Ordering;

use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;

/// An alloc-bit is required per min-object-size aligned address , rather than per object, and can only exist as side metadata.
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::ALLOC_BIT;

pub const ALLOC_SIDE_METADATA_ADDR: Address = ALLOC_SIDE_METADATA_SPEC.get_absolute_offset();

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
    ALLOC_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 1, Ordering::SeqCst);
}

pub fn unset_addr_alloc_bit(address: Address) {
    debug_assert!(
        is_alloced_object(address),
        "{:x}: alloc bit not set",
        address
    );
    ALLOC_SIDE_METADATA_SPEC.store_atomic::<u8>(address, 0, Ordering::SeqCst);
}

pub fn unset_alloc_bit(object: ObjectReference) {
    debug_assert!(is_alloced(object), "{:x}: alloc bit not set", object);
    ALLOC_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 0, Ordering::SeqCst);
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
///
pub unsafe fn unset_alloc_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_alloced(object), "{:x}: alloc bit not set", object);
    ALLOC_SIDE_METADATA_SPEC.store::<u8>(object.to_address(), 0);
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

pub fn is_alloced_object(address: Address) -> bool {
    ALLOC_SIDE_METADATA_SPEC.load_atomic::<u8>(address, Ordering::SeqCst) == 1
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
///
pub unsafe fn is_alloced_object_unsafe(address: Address) -> bool {
    ALLOC_SIDE_METADATA_SPEC.load::<u8>(address) == 1
}

pub fn bzero_alloc_bit(start: Address, size: usize) {
    ALLOC_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}
