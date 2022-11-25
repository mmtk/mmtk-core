use atomic::Ordering;

use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

/// An alloc-bit is required per min-object-size aligned address , rather than per object, and can only exist as side metadata.
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::ALLOC_BIT;

pub const ALLOC_SIDE_METADATA_ADDR: Address = ALLOC_SIDE_METADATA_SPEC.get_absolute_offset();

/// Atomically set the alloc bit for an object.
pub fn set_alloc_bit<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(
        !is_alloced::<VM>(object),
        "{:x}: alloc bit already set",
        object
    );
    ALLOC_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address::<VM>(), 1, Ordering::SeqCst);
}

/// Atomically unset the alloc bit for an object.
pub fn unset_alloc_bit<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(is_alloced::<VM>(object), "{:x}: alloc bit not set", object);
    ALLOC_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address::<VM>(), 0, Ordering::SeqCst);
}

/// Non-atomically unset the alloc bit for an object. The caller needs to ensure the side
/// metadata for the alloc bit for the object is accessed by only one thread.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
pub unsafe fn unset_alloc_bit_unsafe<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(is_alloced::<VM>(object), "{:x}: alloc bit not set", object);
    ALLOC_SIDE_METADATA_SPEC.store::<u8>(object.to_address::<VM>(), 0);
}

/// Check if the alloc bit is set for an object.
pub fn is_alloced<VM: VMBinding>(object: ObjectReference) -> bool {
    ALLOC_SIDE_METADATA_SPEC.load_atomic::<u8>(object.to_address::<VM>(), Ordering::SeqCst) == 1
}

/// Check if an address can be turned directly into an object reference using the alloc bit.
/// If so, return `Some(object)`. Otherwise return `None`.
#[inline]
pub fn is_alloced_object<VM: VMBinding>(address: Address) -> Option<ObjectReference> {
    let potential_object = ObjectReference::from_raw_address(address);
    let addr = potential_object.to_address::<VM>();

    // If we haven't mapped alloc bit for the address, it cannot be an object
    if !ALLOC_SIDE_METADATA_SPEC.is_mapped(addr) {
        return None;
    }

    if ALLOC_SIDE_METADATA_SPEC.load_atomic::<u8>(addr, Ordering::SeqCst) == 1 {
        Some(potential_object)
    } else {
        None
    }
}

/// Check if an address can be turned directly into an object reference using the alloc bit.
/// If so, return `Some(object)`. Otherwise return `None`. The caller needs to ensure the side
/// metadata for the alloc bit for the object is accessed by only one thread.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
#[inline]
pub unsafe fn is_alloced_object_unsafe<VM: VMBinding>(address: Address) -> Option<ObjectReference> {
    let potential_object = ObjectReference::from_raw_address(address);
    let addr = potential_object.to_address::<VM>();

    // If we haven't mapped alloc bit for the address, it cannot be an object
    if !ALLOC_SIDE_METADATA_SPEC.is_mapped(addr) {
        return None;
    }

    if ALLOC_SIDE_METADATA_SPEC.load::<u8>(addr) == 1 {
        Some(potential_object)
    } else {
        None
    }
}

/// Bulk zero the alloc bit.
pub fn bzero_alloc_bit(start: Address, size: usize) {
    ALLOC_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}
