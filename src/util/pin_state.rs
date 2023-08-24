//! This module provides a per-object pinning state which VM bindings can use to prevent the object
//! from being moved by the GC, but does not prevent it from being reclaimed (i.e. does not keep
//! the object alive).
//!
//! # Pinning state
//!
//! This module is enabled by the Cargo feature "object_pinning".  When enabled, each object will
//! have an associated pinning state which can be true or false.  If the state is true, the garbage
//! collector will not move the object.  But if the garbage collector decides that the object is
//! dead, it may still reclaim the object regardless of the pinning state.
//!
//! For spaces that never move objects, including `MarkSweepSpace` and `ImmortalSpace`, the pinning
//! state is always true; for spaces that does not support object pinning, such as `CopySpace`, the
//! pinning state is always false.  For spaces that supports object pinning, such as `ImmixSpace`,
//! the pinning state can be set and unset using the `set_pinned` and `unset_pinned` functions
//! provided by this module.
//!
//! Under the hood, the pin state may be (but not necessarily) implemented by the local PIN_BIT
//! side metadata defined in [`util::metadata::pin_bit`].
//!
//! # Alternative object pinning mechanisms
//!
//! TODO: Update comment after https://github.com/mmtk/mmtk-core/pull/897 is merged
//!

use crate::{mmtk::SFT_MAP, util::address::ObjectReference, vm::VMBinding};

/// Pin an object. MMTk will make sure that the object does not move during GC. Note that action
/// cannot happen in some plans, eg, semispace.
///
/// Arguments:
/// * `object`: The object to be pinned
///
/// It returns true if the pinning operation has been performed, i.e., the object status changed
/// from non-pinned to pinned.
pub fn set_pinned<VM: VMBinding>(object: ObjectReference) -> bool {
    SFT_MAP
        .get_checked(object.to_address::<VM>())
        .set_pinned(object)
}

/// Unpin an object.
///
/// Arguments:
/// * `object`: The object to be pinned
///
/// Returns true if the unpinning operation has been performed, i.e., the object status changed
/// from pinned to non-pinned.
pub fn unset_pinned<VM: VMBinding>(object: ObjectReference) -> bool {
    SFT_MAP
        .get_checked(object.to_address::<VM>())
        .unset_pinned(object)
}

/// Get the pinning state of an object.  Used only for debug purpose.
///
/// **WARNING: Users should not use this function to decide whether it needs to pin or unpin an
/// object.**  In a multi-threaded environment, one thread may change the pinning state during the
/// gap between when another thread reads the pinning state and when that thread takes action
/// accordingly.  This is known as the "time-of-check to time-of-use" (TOC-TOU) problem.  The VM
/// binding should introduce its own synchronisation mechanism when using this module.
///
/// Arguments:
/// * `object`: The object to be checked
///
/// Return true if the objet is pinned.
pub fn debug_get_pinned<VM: VMBinding>(object: ObjectReference) -> bool {
    SFT_MAP
        .get_checked(object.to_address::<VM>())
        .debug_get_pinned(object)
}
