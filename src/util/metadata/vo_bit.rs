//! Valid-object bit (VO-bit)
//!
//! The valid-object bit (VO-bit) metadata is a one-bit-per-object side metadata.  It is set for
//! every object at allocation time (more precisely, during `post_alloc`), and cleared when either
//! -   the object reclaims by the GC, or
//! -   the VM explicitly clears the VO-bit of the object.
//!
//! The main purpose of VO-bit is supporting conservative GC.  It is the canonical source of
//! information about whether there is an object in the MMTk heap at any given address.

use atomic::Ordering;

use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;

/// A VO-bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
pub(crate) const VO_BIT_SIDE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::VO_BIT;

pub const VO_BIT_SIDE_METADATA_ADDR: Address = VO_BIT_SIDE_METADATA_SPEC.get_absolute_offset();

pub fn set_vo_bit(object: ObjectReference) {
    debug_assert!(!is_vo_bit_set(object), "{:x}: VO-bit already set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 1, Ordering::SeqCst);
}

pub fn unset_vo_bit_for_addr(address: Address) {
    debug_assert!(
        is_vo_bit_set_for_addr(address),
        "{:x}: VO-bit not set",
        address
    );
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(address, 0, Ordering::SeqCst);
}

pub fn unset_vo_bit(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 0, Ordering::SeqCst);
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
///
pub unsafe fn unset_vo_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store::<u8>(object.to_address(), 0);
}

pub fn is_vo_bit_set(object: ObjectReference) -> bool {
    is_vo_bit_set_for_addr(object.to_address())
}

pub fn is_vo_bit_set_for_addr(address: Address) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(address, Ordering::SeqCst) == 1
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
///
pub unsafe fn is_vo_bit_set_for_addr_unsafe(address: Address) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load::<u8>(address) == 1
}

pub fn bzero_vo_bit(start: Address, size: usize) {
    VO_BIT_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}
