//! Valid-object bit (VO-bit)
//!
//! The valid-object bit (VO-bit) metadata is a one-bit-per-object side metadata.  It is set for
//! every object at allocation time (more precisely, during `post_alloc`), and cleared when either
//! -   the object reclaims by the GC, or
//! -   the VM explicitly clears the VO-bit of the object.
//!
//! The main purpose of VO-bit is supporting conservative GC.  It is the canonical source of
//! information about whether there is an object in the MMTk heap at any given address.
//!
//! The VO-bit has the granularity of one bit per minimum object alignment.  Each bit governs the
//! region of `lo <= addr < hi`, where
//! -   `lo = addr.align_down(VO_BIT_REGION_SIZE)`
//! -   `hi = lo + VO_BIT_REGION_SIZE`
//! -   The constant [`VO_BIT_REGION_SIZE`] is size of the region (in bytes) each bit governs.
//!
//! Because of the granularity, if the user wants to check if an *arbitrary* address points to a
//! valid object, it must check if the address is properly aligned.

use atomic::Ordering;

#[cfg(feature = "vo_bit")]
// Eventually the entire `vo_bit` module will be guarded by this feature.
use crate::mmtk::SFT_MAP;
#[cfg(feature = "vo_bit")]
// Eventually the entire `vo_bit` module will be guarded by this feature.
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

pub(crate) fn set_vo_bit(object: ObjectReference) {
    debug_assert!(!is_vo_bit_set(object), "{:x}: VO-bit already set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 1, Ordering::SeqCst);
}

pub(crate) fn unset_vo_bit(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store_atomic::<u8>(object.to_address(), 0, Ordering::SeqCst);
}

pub(crate) fn is_vo_bit_set(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load_atomic::<u8>(object.to_address(), Ordering::SeqCst) == 1
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::store`
///
pub(crate) unsafe fn unset_vo_bit_unsafe(object: ObjectReference) {
    debug_assert!(is_vo_bit_set(object), "{:x}: VO-bit not set", object);
    VO_BIT_SIDE_METADATA_SPEC.store::<u8>(object.to_address(), 0);
}

/// # Safety
///
/// This is unsafe: check the comment on `side_metadata::load`
///
pub(crate) unsafe fn is_vo_bit_set_unsafe(object: ObjectReference) -> bool {
    VO_BIT_SIDE_METADATA_SPEC.load::<u8>(object.to_address()) == 1
}

pub(crate) fn bzero_vo_bit(start: Address, size: usize) {
    VO_BIT_SIDE_METADATA_SPEC.bzero_metadata(start, size);
}

/// Check if `object` is a reference to a valid MMTk object.
///
/// Concretely:
/// 1.  Return true if `addr.to_object_reference()` is a valid object reference to an object in any
///     space in MMTk.
/// 2.  Also return true if there exists an `objref: ObjectReference` such that
///     -   `objref` is a valid object reference to an object in any space in MMTk, and
///     -   `lo <= objref.to_address() < hi`, where
///         -   `lo = addr.align_down(VO_BIT_REGION_SIZE)` and
///         -   `hi = lo + VO_BIT_REGION_SIZE` and
///         -   `VO_BIT_REGION_SIZE` is [`crate::util::is_mmtk_object::VO_BIT_REGION_SIZE`].
///             It is the byte granularity of the VO-bit.
/// 3.  Return false otherwise.  This function never panics.
///
/// Case 2 means **this function is imprecise for misaligned addresses**.
/// This function uses the VO-bit side metadata, i.e. a bitmap.
/// For space efficiency, each bit of the bitmap governs a small region of memory.
/// The size of a region is currently defined as the [minimum object size](crate::util::constants::MIN_OBJECT_SIZE),
/// which is currently defined as the [word size](crate::util::constants::BYTES_IN_WORD),
/// which is 4 bytes on 32-bit systems or 8 bytes on 64-bit systems.
/// The alignment of a region is also the region size.
/// If a VO-bit is `1`, the bitmap cannot tell which address within the 4-byte or 8-byte region
/// is the valid object reference.
/// Therefore, if the input `addr` is not properly aligned, but is close to a valid object
/// reference, this function may still return true.
///
/// For the reason above, the VM **must check if `addr` is properly aligned** before calling this
/// function.  For most VMs, valid object references are always aligned to the word size, so
/// checking `addr.is_aligned_to(BYTES_IN_WORD)` should usually work.  If you are paranoid, you can
/// always check against [`crate::util::is_mmtk_object::VO_BIT_REGION_SIZE`].
///
/// This function is useful for conservative root scanning.  The VM can iterate through all words in
/// a stack, filter out zeros, misaligned words, obviously out-of-range words (such as addresses
/// greater than `0x0000_7fff_ffff_ffff` on Linux on x86_64), and use this function to deside if the
/// word is really a reference.
///
/// Note: This function has special behaviors if the VM space (enabled by the `vm_space` feature)
/// is present.  See `crate::plan::global::BasePlan::vm_space`.
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
pub fn invalidate_object(object: ObjectReference) {
    debug_assert!(SFT_MAP
        .get_checked(object.to_address())
        .is_valid_mmtk_object(object));
    unset_vo_bit(object);
}
