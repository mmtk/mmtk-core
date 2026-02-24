//! This module is for allocating large arrays or vectors with initial zero values.
//!
//! Currently we use the [`Zeroable`] trait from the `bytemuck` crate to label types that are safe
//! for zero initialization.  If one day Rust provides a standard way to optimize for zeroed
//! allocation of vectors of composite types, we can switch to the standard mechanism.
//!
//! Note: The standard library uses the `IsZero` trait to specialize the intialization of `Vec<T>`
//! if the initial element values are zero.  Primitive type, such as `i8`, `usize`, `f32`, as well
//! as types with known representations such as `Option<NonZeroUsize>` implement the `IsZero` trait.
//! However, it has several limitations.
//!
//! 1.  Composite types, such as `SpaceDescriptor(usize)`, don't implement the `IsZero` trait, even
//!     if they have the `#[repr(transparent)]` annotation.
//! 2.  The `IsZero` trait is private to the `std` module, and we cannot use it.
//!
//! Therefore, `vec![0usize; 33554432]` takes only 4 **microseconds**, while
//! `vec![SpaceDescriptor(0); 33554432]` will take 22 **milliseconds** to execute on some machine.
//! If such an allocation happens during start-up, the delay will be noticeable to light-weight
//! scripting languages, such as Ruby.
//!
//! The [`new_zeroed_vec`] function in this module can allocate zeroed vectors as fast as `vec![0;
//! LEN]`;
use std::alloc::{alloc_zeroed, handle_alloc_error, Layout};

use bytemuck::Zeroable;

/// Allocate a `Vec<T>` of all-zero values.  `T` must implement [`bytemuck::Zeroable`].
///
/// This intends to be a faster alternative to `vec![T(0), size]`.  It will allocate pre-zeroed
/// buffer, and not store zero values to its elements as part of initialization.
///
/// It is useful when creating large (hundreds of megabytes) Vecs when the execution time is
/// critical (such as during start-up, where a 100ms delay is obvious to small applications.)
///
/// Arguments:
///
/// -   `T`: The element type.
/// -   `size`: The length and capacity of the created vector.
///
/// Returns the created vector.
pub(crate) fn new_zeroed_vec<T: Zeroable>(size: usize) -> Vec<T> {
    let layout = Layout::array::<T>(size).unwrap();
    let ptr = unsafe { alloc_zeroed(layout) } as *mut T;
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    unsafe { Vec::from_raw_parts(ptr, size, size) }
}
