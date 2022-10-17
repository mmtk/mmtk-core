//! Valid-object bit (VO-bit)
//!
//! The valid-object bit (VO-bit) metadata is a one-bit-per-object side metadata.  It is set for
//! every object at allocation time (more precisely, during `post_alloc`), and cleared when either
//! -   the object is reclaimed by the GC, or
//! -   the VM explicitly clears the VO-bit of the object (using the [`invalidate_object`] API).
//!
//! The main purpose of VO-bit is supporting conservative GC.  It is the canonical source of
//! information about whether there is an object in the MMTk heap at any given address.
//!
//! The granularity of VO-bit is one bit per minimum object alignment.  Each bit governs the
//! region of `lo <= addr < hi`, where
//! -   `lo = addr.align_down(VO_BIT_REGION_SIZE)`
//! -   `hi = lo + VO_BIT_REGION_SIZE`
//! -   The constant [`VO_BIT_REGION_SIZE`] is size of the region (in bytes) each bit governs.
//!
//! Because of the granularity, the VO-bit metadata cannot tell *which* address in each region
//! has a valid object.  Therefore, the VM **must check if an address is properly aligned** before
//! consulting the VO-bit metadata (by calling the [`is_valid_mmtk_object`] function).  For most
//! VMs, the alignment requirement of object references is usually equal to [`VO_BIT_REGION_SIZE`],
//! so checking `object.to_address().is_aligned_to(VO_BIT_REGION_SIZE)` should usually work.
//!
//! This function is useful for conservative root scanning.  The VM can iterate through all words
//! in a stack, filter out zeros, misaligned words, obviously out-of-range words (such as addresses
//! greater than `0x0000_7fff_ffff_ffff` on Linux on x86_64), and use this function to deside if
//! the word is really a reference.
//!
//! Note: This function has special behaviors if the VM space (enabled by the `vm_space` feature)
//! is present.  See `crate::plan::global::BasePlan::vm_space`.
//!

use atomic::Ordering;

#[cfg(feature = "vo_bit")]
use crate::mmtk::SFT_MAP;
#[cfg(feature = "vo_bit")]
use crate::policy::sft_map::SFTMap;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;

/// A VO-bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
pub(crate) const VO_BIT_SIDE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::VO_BIT;

pub(crate) const VO_BIT_SIDE_METADATA_ADDR: Address =
    VO_BIT_SIDE_METADATA_SPEC.get_absolute_offset();

/// The region size (in bytes) of the `VO_BIT` side metadata.
///
/// Currently, it is set to the [minimum object size](crate::util::constants::MIN_OBJECT_SIZE),
/// which is currently defined as the [word size](crate::util::constants::BYTES_IN_WORD).
///
/// The VM can use this to check if an object is properly aligned.
#[cfg(feature = "vo_bit")] // Eventually the entire `vo_bit` module will be guarded by this feature.
pub const VO_BIT_REGION_SIZE: usize =
    1usize << crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC.log_bytes_in_region;

/// Set the VO-bit of `object` atomically.
pub(crate) fn set_vo_bit(object: ObjectReference) {
    debug_assert!(!is_vo_bit_set(object), "{:x}: VO-bit already set", object);
    VO_BIT_SIDE_METADATA_SPEC.fetch_or_atomic::<u8>(object.to_address(), 1, Ordering::SeqCst);
}

/// Unset the VO-bit of `object` atomically.
pub(crate) fn unset_vo_bit(object: ObjectReference) {
    // Note: Both the VM and the GC are allowed to unset VO-bit.  However, if the VM unsets the
    // VO-bit first, that object will not be traced by the GC, and the GC will not try to clear its
    // VO-bit again.  So it is valid to assert the VO-bit must still be set when this function is
    // called.
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.fetch_and_atomic::<u8>(object.to_address(), 0, Ordering::SeqCst);
}

/// Check if the VO-bit of `object` is set atomically.
pub(crate) fn is_vo_bit_set(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(object.to_address(), Ordering::SeqCst) == 1
}

/// Unset the VO-bit of `object` non-atomically.
///
/// # Safety
///
/// It will be a data race if another thread concurrently accesses any bit in the/ same byte.
/// It should only be used when such a race is impossible.
pub(crate) unsafe fn unset_vo_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store::<u8>(object.to_address(), 0);
}

/// Check if the VO-bit of `object` is set non-atomically.
///
/// # Safety
///
/// It will be a data race if another thread concurrently modifies any bit in the same byte.
/// It should only be used when such a race is impossible.
pub(crate) unsafe fn is_vo_bit_set_unsafe(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load::<u8>(object.to_address()) == 1
}

/// Unset all VO-bits for all objects in the region of `start <= addr < start + size`.
///
/// # Safety
///
/// It will be a data race if another thread concurrently accesses any bit in the region.
/// It should only be used when such a race is impossible.
pub(crate) unsafe fn bzero_vo_bit(start: Address, size: usize) {
    VO_BIT_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}

/// Check if `object` is a reference to a valid MMTk object.
///
/// This function returns true if the VO-bit is set for the address of `object`.
///
/// The input parameter `object` can be converted from an arbitrary address.  This function will
/// always return true or false, and will never panic.
///
/// Due to the granularity of the VO-bit metadata (see [module-level documentation][self]), the
/// user must check the alignment of `object` before calling this function in order to get the
/// correct result.
///
/// Argument:
/// * `object`: An ObjectReference converted from an arbitrary address
#[cfg(feature = "vo_bit")] // Eventually the entire `vo_bit` module will be guarded by this feature.
pub fn is_valid_mmtk_object(object: ObjectReference) -> bool {
    SFT_MAP
        .get_checked(object.to_address())
        .is_valid_mmtk_object(object)
}

/// Invalidate an object.
///
/// By calling this method, the GC will treat it as dead, and will not trace it or scan it in
/// subsequent GCs.  It is useful for VMs that may destory per-object metadata (ususally during
/// finalization) so that any attempt to scan the object after that may result in crash.
///
/// Argument:
/// * `object`: An object that is still valid.
#[cfg(feature = "vo_bit")] // Eventually the entire `vo_bit` module will be guarded by this feature.
pub fn invalidate_object(_object: ObjectReference) {
    unimplemented!("GC algorithms need to be updated to skip invalid objects during tracing.")
}
