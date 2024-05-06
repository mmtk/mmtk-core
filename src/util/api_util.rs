//! This module contain helpers for the convenience of exposing the MMTk API to native (usually
//! C/C++) programs.

use super::{Address, ObjectReference};

/// An `Option<ObjectReference>` encoded as a `usize` (which is guaranteed to have the size of a
/// native pointer).  It guarantees that `None` is encoded as 0, and `Some(objref)` is encoded as
/// the underlying `usize` value of the `ObjectReference` itself.
///
/// Note: The Rust ABI currently doesn't guarantee the encoding of `None` even if the `T` in
/// `Option<T>` is eligible for null pointer optimization.  Transmuting a `None` value of
/// `Option<ObjectReference>` to `usize` still has undefined behavior.
/// See: <https://doc.rust-lang.org/std/option/index.html#representation>
///
/// It is intended for passing an `Option<ObjectReference>` values to and from native programs
/// (usually C or C++) that have null pointers.
#[repr(transparent)]
pub struct NullableObjectReference(usize);

impl From<NullableObjectReference> for Option<ObjectReference> {
    fn from(value: NullableObjectReference) -> Self {
        ObjectReference::from_raw_address(unsafe { Address::from_usize(value.0) })
    }
}

impl From<Option<ObjectReference>> for NullableObjectReference {
    fn from(value: Option<ObjectReference>) -> Self {
        let encoded = value
            .map(|obj| obj.to_raw_address().as_usize())
            .unwrap_or(0);
        Self(encoded)
    }
}
