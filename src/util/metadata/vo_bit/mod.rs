//! Valid object bit (VO bit)
//!
//! The valid object bit, or "VO bit" for short", is a global per-address metadata.  It is set at
//! the address of the `ObjectReference` of an object when the object is allocated, and cleared
//! when the object is determined to be dead by the GC.
//!
//! The VO bit metadata serves multiple purposes, including but not limited to:
//!
//! | purpose                                     | happens when                                  |
//! |---------------------------------------------|-----------------------------------------------|
//! | conservative stack scanning                 | stack scanning                                |
//! | conservative object scanning                | tracing                                       |
//! | supporting interior pointers                | tracing                                       |
//! | heap dumping (by tracing)                   | tracing                                       |
//! | heap dumping (by iteration)                 | before or after tracing                       |
//! | heap iteration (for GC algorithm)           | depending on algorithm                        |
//! | heap iteration (for VM API, e.g. JVM-TI)    | during mutator time                           |
//! | sanity checking                             | any time in GC                                |
//!
//! Among the listed purposes, conservative stack scanning and conservative objects scanning are
//! visible to the VM binding.  By default, if the "vo_bit" cargo feature is enabled, the VO bits
//! metadata will be available to the VM binding during stack scanning time.  The VM binding can
//! further require the VO bits to be available during tracing (for object scanning) by setting
//! [`crate::vm::ObjectModel::NEED_VO_BITS_DURING_TRACING`] to `true`.  mmtk-core does not
//! guarantee the VO bits are available to the VM binding during other time.
//!
//! Internally, mmtk-core will also make the VO bits available when necessary if mmtk-core needs to
//! implement features that needs VO bits.
//!
//! When the VO bits are available during tracing, if a plan uses evacuation to reclaim space, then
//! both the from-space copy and the to-space copy of an object will have the VO-bit set.
//!
//! *(Note: There are several reasons behind this semantics.  One reason is that a slot may be
//! visited multiple times during GC.  If a slot is visited twice, we will see the object reference
//! in the slot pointing to the from-space copy during the first visit, but pointing to the to-space
//! copy during the second visit.  We consider an object reference valid if it points to either the
//! from-space or the to-space copy.  If each slot is visited only once, and we see a slot happen to
//! hold a pointer into the to-space during its only visit, that must be a dangling pointer, and
//! error should be reported.  However, it is hard to guarantee each slot is only visited once
//! during tracing because both the VM and the GC algorithm may break this guarantee.  See:
//! [`crate::plan::PlanConstraints::may_trace_duplicate_edges`])*

// FIXME: The entire vo_bit module should only be available if the "vo_bit" feature is enabled.
// However, the malloc-based MarkSweepSpace and MarkCompactSpace depends on the VO bits regardless
// of the "vo_bit" feature.
#[cfg(feature = "vo_bit")]
pub(crate) mod helper;

use atomic::Ordering;

use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;

cfg_if::cfg_if! {
    if #[cfg(feature = "vo_bit_access")] {
        /// A VO bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
        /// This is only publicly available when the feature "vo_bit_access" is enabled.
        /// Check the comments on "vo_bit_access" in `Cargo.toml` before use. Use at your own risk.
        pub const VO_BIT_SIDE_METADATA_SPEC: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::VO_BIT;
    } else {
        /// A VO bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
        pub(crate) const VO_BIT_SIDE_METADATA_SPEC: SideMetadataSpec =
            crate::util::metadata::side_metadata::spec_defs::VO_BIT;
    }
}

/// The base address for VO bit side metadata on 64 bits platforms.
#[cfg(target_pointer_width = "64")]
pub fn vo_bit_side_metadata_addr() -> Address {
    VO_BIT_SIDE_METADATA_SPEC.get_absolute_offset()
}

/// Atomically set the VO bit for an object.
pub(crate) fn set_vo_bit(object: ObjectReference) {
    debug_assert!(!is_vo_bit_set(object), "{:x}: VO bit already set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_raw_address(), 1, Ordering::SeqCst);
}

/// Atomically unset the VO bit for an object.
pub(crate) fn unset_vo_bit(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_raw_address(), 0, Ordering::SeqCst);
}

/// Atomically unset the VO bit for an object, regardless whether the bit is set or not.
pub(crate) fn unset_vo_bit_nocheck(object: ObjectReference) {
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_raw_address(), 0, Ordering::SeqCst);
}

/// Non-atomically unset the VO bit for an object. The caller needs to ensure the side
/// metadata for the VO bit for the object is accessed by only one thread.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
pub(crate) unsafe fn unset_vo_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store::<u8>(object.to_raw_address(), 0);
}

/// Check if the VO bit is set for an object.
pub(crate) fn is_vo_bit_set(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(object.to_raw_address(), Ordering::SeqCst) == 1
}

/// Check if an address can be turned directly into an object reference using the VO bit.
/// If so, return `Some(object)`. Otherwise return `None`.
///
/// The `address` must be word-aligned.
pub(crate) fn is_vo_bit_set_for_addr(address: Address) -> Option<ObjectReference> {
    is_vo_bit_set_inner::<true>(address)
}

/// Check if an address can be turned directly into an object reference using the VO bit.
/// If so, return `Some(object)`. Otherwise return `None`. The caller needs to ensure the side
/// metadata for the VO bit for the object is accessed by only one thread.
///
/// The `address` must be word-aligned.
///
/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
pub(crate) unsafe fn is_vo_bit_set_unsafe(address: Address) -> Option<ObjectReference> {
    is_vo_bit_set_inner::<false>(address)
}

fn is_vo_bit_set_inner<const ATOMIC: bool>(addr: Address) -> Option<ObjectReference> {
    debug_assert!(
        addr.is_aligned_to(ObjectReference::ALIGNMENT),
        "Address is not word-aligned: {addr}"
    );

    // If we haven't mapped VO bit for the address, it cannot be an object
    if !VO_BIT_SIDE_METADATA_SPEC.is_mapped(addr) {
        return None;
    }

    let vo_bit = if ATOMIC {
        VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(addr, Ordering::SeqCst)
    } else {
        unsafe { VO_BIT_SIDE_METADATA_SPEC.load::<u8>(addr) }
    };

    (vo_bit == 1).then(|| get_object_ref_for_vo_addr(addr))
}

/// Bulk zero the VO bit.
pub(crate) fn bzero_vo_bit(start: Address, size: usize) {
    VO_BIT_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}

/// Bulk copy VO bits from side mark bits.
/// Some VMs require the VO bits to be available during tracing.
/// However, some GC algorithms (such as Immix) cannot clear VO bits for dead objects only.
/// As an alternative, this function copies the mark bits metadata to VO bits.
/// The caller needs to ensure the mark bits are set exactly wherever VO bits need to be set before
/// calling this function.
pub(crate) fn bcopy_vo_bit_from_mark_bit<VM: VMBinding>(start: Address, size: usize) {
    let mark_bit_spec = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC;
    debug_assert!(
        mark_bit_spec.is_on_side(),
        "bcopy_vo_bit_from_mark_bits can only be used with on-the-side mark bits."
    );
    let side_mark_bit_spec = mark_bit_spec.extract_side_spec();
    VO_BIT_SIDE_METADATA_SPEC.bcopy_metadata_contiguous(start, size, side_mark_bit_spec);
}

use crate::util::constants::{LOG_BITS_IN_BYTE, LOG_BYTES_IN_ADDRESS};

/// How many data memory bytes does 1 word in the VO bit side metadata represents?
pub(crate) const VO_BIT_WORD_TO_REGION: usize = 1
    << (VO_BIT_SIDE_METADATA_SPEC.log_bytes_in_region
        + LOG_BITS_IN_BYTE as usize
        + LOG_BYTES_IN_ADDRESS as usize
        - VO_BIT_SIDE_METADATA_SPEC.log_num_of_bits);

/// Bulk check if a VO bit word. Return true if there is any bit set in the word.
pub(crate) fn get_raw_vo_bit_word(addr: Address) -> usize {
    unsafe { VO_BIT_SIDE_METADATA_SPEC.load_raw_word(addr) }
}

/// Find the base reference to the object from a potential internal pointer.
pub(crate) fn find_object_from_internal_pointer<VM: VMBinding>(
    start: Address,
    search_limit_bytes: usize,
) -> Option<ObjectReference> {
    if !start.is_mapped() {
        return None;
    }

    if let Some(vo_addr) = unsafe {
        VO_BIT_SIDE_METADATA_SPEC.find_prev_non_zero_value::<u8>(start, search_limit_bytes)
    } {
        is_internal_ptr_from_vo_bit::<VM>(vo_addr, start)
    } else {
        None
    }
}

/// Get the object reference from an aligned address where VO bit is set.
pub(crate) fn get_object_ref_for_vo_addr(vo_addr: Address) -> ObjectReference {
    // VO bit should be set on the address.
    debug_assert!(vo_addr.is_aligned_to(ObjectReference::ALIGNMENT));
    debug_assert!(unsafe { is_vo_addr(vo_addr) });
    unsafe { ObjectReference::from_raw_address_unchecked(vo_addr) }
}

/// Check if the address could be an internal pointer in the object.
fn is_internal_ptr<VM: VMBinding>(obj: ObjectReference, internal_ptr: Address) -> bool {
    let obj_start = obj.to_object_start::<VM>();
    let obj_size = VM::VMObjectModel::get_current_size(obj);
    internal_ptr < obj_start + obj_size
}

/// Check if the address could be an internal pointer based on where VO bit is set.
pub(crate) fn is_internal_ptr_from_vo_bit<VM: VMBinding>(
    vo_addr: Address,
    internal_ptr: Address,
) -> Option<ObjectReference> {
    let obj = get_object_ref_for_vo_addr(vo_addr);
    if is_internal_ptr::<VM>(obj, internal_ptr) {
        Some(obj)
    } else {
        None
    }
}

/// Non-atomically check if the VO bit is set for this address.
///
/// # Safety
/// The caller needs to make sure that no one is modifying VO bit.
pub(crate) unsafe fn is_vo_addr(addr: Address) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load::<u8>(addr) != 0
}
