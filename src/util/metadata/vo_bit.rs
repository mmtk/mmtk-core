//! Valid object bit (VO bit)
//!
//! The valid object bit, or "VO bit" for short", is a global per-address metadata.  It is set at
//! the address of the `ObjectReference` of an object when the object is allocated, and cleared
//! when the object is reclaimed by the GC.
//!
//! The main purpose of VO bit is supporting conservative GC.  It is the canonical source of
//! information about whether there is an object in the MMTk heap at any given address.

use atomic::Ordering;

use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;

/// A VO bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
pub(crate) const VO_BIT_SIDE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::VO_BIT;

pub const VO_BIT_SIDE_METADATA_ADDR: Address = VO_BIT_SIDE_METADATA_SPEC.get_absolute_offset();

/// Atomically set the VO bit for an object.
pub fn set_vo_bit<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(
        !is_vo_bit_set::<VM>(object),
        "{:x}: VO bit already set",
        object
    );
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address::<VM>(), 1, Ordering::SeqCst);
}

/// Atomically unset the VO bit for an object.
pub fn unset_vo_bit<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(is_vo_bit_set::<VM>(object), "{:x}: VO bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address::<VM>(), 0, Ordering::SeqCst);
}

/// Atomically unset the VO bit for an object, regardless whether the bit is set or not.
pub fn unset_vo_bit_nocheck<VM: VMBinding>(object: ObjectReference) {
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address::<VM>(), 0, Ordering::SeqCst);
}

/// Non-atomically unset the VO bit for an object. The caller needs to ensure the side
/// metadata for the VO bit for the object is accessed by only one thread.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
pub unsafe fn unset_vo_bit_unsafe<VM: VMBinding>(object: ObjectReference) {
    debug_assert!(is_vo_bit_set::<VM>(object), "{:x}: VO bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store::<u8>(object.to_address::<VM>(), 0);
}

/// Check if the VO bit is set for an object.
pub fn is_vo_bit_set<VM: VMBinding>(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(object.to_address::<VM>(), Ordering::SeqCst) == 1
}

/// Check if an address can be turned directly into an object reference using the VO bit.
/// If so, return `Some(object)`. Otherwise return `None`.
pub fn is_vo_bit_set_for_addr<VM: VMBinding>(address: Address) -> Option<ObjectReference> {
    let potential_object = ObjectReference::from_raw_address(address);
    let addr = potential_object.to_address::<VM>();

    // If we haven't mapped VO bit for the address, it cannot be an object
    if !VO_BIT_SIDE_METADATA_SPEC.is_mapped(addr) {
        return None;
    }

    if VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(addr, Ordering::SeqCst) == 1 {
        Some(potential_object)
    } else {
        None
    }
}

/// Check if an address can be turned directly into an object reference using the VO bit.
/// If so, return `Some(object)`. Otherwise return `None`. The caller needs to ensure the side
/// metadata for the VO bit for the object is accessed by only one thread.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
pub unsafe fn is_vo_bit_set_unsafe<VM: VMBinding>(address: Address) -> Option<ObjectReference> {
    let potential_object = ObjectReference::from_raw_address(address);
    let addr = potential_object.to_address::<VM>();

    // If we haven't mapped VO bit for the address, it cannot be an object
    if !VO_BIT_SIDE_METADATA_SPEC.is_mapped(addr) {
        return None;
    }

    if VO_BIT_SIDE_METADATA_SPEC.load::<u8>(addr) == 1 {
        Some(potential_object)
    } else {
        None
    }
}

/// Bulk zero the VO bit.
pub fn bzero_vo_bit(start: Address, size: usize) {
    VO_BIT_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}

/// Bulk copy VO bits from side mark bits.
/// Some VMs require the VO bits to be available during tracing.
/// However, some GC algorithms (such as Immix) cannot clear VO bits for dead objects only.
/// As an alternative, this function copies the mark bits metadata to VO bits.
/// The caller needs to ensure the mark bits are set exactly wherever VO bits need to be set before
/// calling this function.
pub fn bcopy_vo_bit_from_mark_bit<VM: VMBinding>(start: Address, size: usize) {
    let mark_bit_spec = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC;
    debug_assert!(
        mark_bit_spec.is_on_side(),
        "bcopy_vo_bit_from_mark_bits can only be used with on-the-side mark bits."
    );
    let side_mark_bit_spec = mark_bit_spec.extract_side_spec();
    VO_BIT_SIDE_METADATA_SPEC.bcopy_metadata_contiguous(start, size, side_mark_bit_spec);
}
