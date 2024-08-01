//! Utilities used by other modules, including allocators, heap implementation, etc.

// Allow unused code in the util mod. We may have some functions that are not in use,
// but will be useful in future implementations.
#![allow(dead_code)]

// The following modules are public. MMTk bindings can use them to help implementation.

/// An abstract of memory address and object reference.
pub mod address;
/// Allocators
// This module is made public so the binding could implement allocator slowpaths if they would like to.
pub mod alloc;
/// Helpers for making native APIs.
pub mod api_util;
/// Constants used in MMTk
pub mod constants;
/// Calculation, conversion and rounding for memory related numbers.
pub mod conversions;
/// The copy allocators for a GC worker.
pub mod copy;
/// Heap implementation, including page resource, mmapper, etc.
pub mod heap;
/// Checking if an address is an valid MMTk object.
#[cfg(feature = "is_mmtk_object")]
pub mod is_mmtk_object;
/// Linear scan through a heap range
pub mod linear_scan;
/// Various malloc implementations (conditionally compiled by features)
pub mod malloc;
/// Wrapper functions for memory syscalls such as mmap, mprotect, etc.
pub mod memory;
/// Metadata (OnSide or InHeader) implementation.
pub mod metadata;
/// Opaque pointers used in MMTk, e.g. VMThread.
pub mod opaque_pointer;
/// MMTk command line options.
pub mod options;
/// Test utilities. We need this module for `MockVM` in criterion benches, which does not include code with `cfg(test)`.
#[cfg(any(test, feature = "mock_test"))]
pub mod test_util;

// The following modules are only public in the mmtk crate. They should only be used in MMTk core.
/// An analysis framework for collecting data and profiling in GC.
#[cfg(feature = "analysis")]
pub(crate) mod analysis;
pub(crate) mod epilogue;
/// Non-generic refs to generic types of `<VM>`.
pub(crate) mod erase_vm;
/// Finalization implementation.
pub(crate) mod finalizable_processor;
/// Logger initialization
pub(crate) mod logger;
/// Forwarding word in object copying.
pub(crate) mod object_forwarding;
/// Reference processing implementation.
pub(crate) mod reference_processor;
/// Utilities funcitons for Rust
pub(crate) mod rust_util;
/// Sanity checker for GC.
#[cfg(feature = "sanity")]
pub(crate) mod sanity;
/// Logging slots to check duplicated edges in GC.
#[cfg(feature = "extreme_assertions")]
pub(crate) mod slot_logger;
/// Utils for collecting statistics.
pub(crate) mod statistics;
/// A treadmill implementation.
pub(crate) mod treadmill;

// These modules are private. They are only used by other util modules.

/// A very simple, generic malloc-free allocator
mod freelist;
/// Implementation of GenericFreeList by an int vector.
mod int_array_freelist;
/// Implementation of GenericFreeList backed by raw memory, allocated
/// on demand direct from the OS (via mmap).
mod raw_memory_freelist;

pub use self::address::Address;
pub use self::address::ObjectReference;
pub use self::opaque_pointer::*;
