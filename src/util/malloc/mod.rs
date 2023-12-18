//! This module exposes a set of malloc API. They are currently implemented with
//! the library malloc. This may change in the future, and will be replaced
//! with a native MMTk implementation.

//! We have two versions for each function:
//! * a normal version: it has the signature that is compatible with the standard malloc library.
//! * a counted version: the allocated/freed bytes are calculated into MMTk's heap. So extra arguments
//!   are needed to maintain allocated bytes properly. The API is inspired by Julia's counted malloc.
//!   The counted version is only available with the feature `malloc_counted_size`.

/// Malloc provided by libraries
pub(crate) mod library;
/// Using malloc as mark sweep free-list allocator.
pub(crate) mod malloc_ms_util;

use crate::util::Address;
#[cfg(feature = "malloc_counted_size")]
use crate::vm::VMBinding;
#[cfg(feature = "malloc_counted_size")]
use crate::MMTK;

/// Manually allocate memory. Similar to libc's malloc.
pub fn malloc(size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::malloc(size) })
}

/// Manually allocate memory. Similar to libc's malloc.
/// This also counts the allocated memory into the heap size of the given MMTk instance.
#[cfg(feature = "malloc_counted_size")]
pub fn counted_malloc<VM: VMBinding>(mmtk: &MMTK<VM>, size: usize) -> Address {
    let res = malloc(size);
    if !res.is_zero() {
        mmtk.state.increase_malloc_bytes_by(size);
    }
    res
}

/// Manually allocate memory and initialize the bytes in the allocated memory to zero. Similar to libc's calloc.
pub fn calloc(num: usize, size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::calloc(num, size) })
}

/// Manually allocate memory and initialize the bytes in the allocated memory to zero. Similar to libc's calloc.
/// This also counts the allocated memory into the heap size of the given MMTk instance.
#[cfg(feature = "malloc_counted_size")]
pub fn counted_calloc<VM: VMBinding>(mmtk: &MMTK<VM>, num: usize, size: usize) -> Address {
    let res = calloc(num, size);
    if !res.is_zero() {
        mmtk.state.increase_malloc_bytes_by(num * size);
    }
    res
}

/// Reallocate the given area of memory. Similar to libc's realloc.
pub fn realloc(addr: Address, size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::realloc(addr.to_mut_ptr(), size) })
}

/// Reallocate the given area of memory. Similar to libc's realloc.
/// This also adjusts the allocated memory size based on the original allocation and the new allocation, and counts
/// that into the heap size for the given MMTk instance.
#[cfg(feature = "malloc_counted_size")]
pub fn realloc_with_old_size<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    addr: Address,
    size: usize,
    old_size: usize,
) -> Address {
    let res = realloc(addr, size);

    if !addr.is_zero() {
        mmtk.state.decrease_malloc_bytes_by(old_size);
    }
    if size != 0 && !res.is_zero() {
        mmtk.state.increase_malloc_bytes_by(size);
    }

    res
}

/// Manually free the memory that is returned from other manual allocation functions in this module.
pub fn free(addr: Address) {
    unsafe { self::library::free(addr.to_mut_ptr()) }
}

/// Manually free the memory that is returned from other manual allocation functions in this module.
/// This also reduces the allocated memory size.
#[cfg(feature = "malloc_counted_size")]
pub fn free_with_size<VM: VMBinding>(mmtk: &MMTK<VM>, addr: Address, old_size: usize) {
    free(addr);
    if !addr.is_zero() {
        mmtk.state.decrease_malloc_bytes_by(old_size);
    }
}
