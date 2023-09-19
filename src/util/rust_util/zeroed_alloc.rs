//! This module is for allocating large arrays or vectors with initial zero values.
//!
//! Note: The standard library uses the `IsZero` trait to specialize the intialization of `Vec<T>`
//! if the initial element values are zero.  Primitive type, such as `i8`, `usize`, `f32`, as well
//! as types with known representations such as `Option<NonZeroUsize>` implement the `IsZero`
//! trait.  However, it has several limitations.
//!
//! 1.  Composite types, such as `SpaceDescriptor(usize)`, doesn't implement the `IsZero` trait,
//!     even if it has the `#[repr(transparent)]` annotation.
//! 2.  The `IsZero` trait is private to the `std` module, and we cannot use it.
//!
//! Therefore, `vec![0usize; 33554432]` takes only 4 **microseconds**, while
//! `vec![SpaceDescriptor(0); 33554432]` will take 22 **milliseconds** to execute on some machine.
//! If such an allocation happens during start-up, the delay will be noticeable to light-weight
//! scripting languages, such as Ruby.
//!
//! *(Note: We no longer allocate such large vecs at start-up.  We keep this module in case we need
//! to allocate large vectors in the future.)*
//!
//! We implement our own fast allocation of large zeroed vectors in this module.  If one day Rust
//! provides a standard way to optimize for zeroed allocation of vectors of composite types, we
//! can switch to the standard mechanism.
use std::alloc::{alloc_zeroed, Layout};

/// Allocate a `Vec<T>` of all-zero values.
///
/// This intends to be a faster alternative to `vec![T(0), size]`.  It will allocate pre-zeroed
/// buffer, and not store zero values to its elements as part of initialization.
///
/// It is useful when creating large (hundreds of megabytes) Vecs when the execution time is
/// critical (such as during start-up, where a 100ms delay is obvious to small applications.)
/// However, because of its unsafe nature, it should only be used when necessary.
///
/// Arguments:
///
/// -   `T`: The element type.
/// -   `size`: The length and capacity of the created vector.
///
/// Returns the created vector.
///
/// # Unsafe
///
/// This function is unsafe.  It will not call any constructor of `T`.  The user must ensure
/// that a value with all bits being zero is meaningful for type `T`.
pub(crate) unsafe fn new_zeroed_vec<T>(size: usize) -> Vec<T> {
    let layout = Layout::array::<T>(size).unwrap();
    let ptr = alloc_zeroed(layout) as *mut T;
    Vec::from_raw_parts(ptr, size, size)
}
