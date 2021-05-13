//! Utilities used by other modules, including allocators, heap implementation, etc.

// Allow unused code in the util mod. We may have some functions that are not in use,
// but will be useful in the future implementation.
#![allow(dead_code)]

// These modules are public. The bindings can use them to help implementation.

/// An abstract of memory address and object reference.
pub mod address;
/// Allocators
// This module is made public so the binding could implement allocator slowpaths if they would like to.
pub mod alloc;
/// Calculation, conversion and rounding for memory related numbers.
pub mod conversions;
// Constants used in MMTk
pub mod constants;
/// Wrapper functions for memory syscalls such as mmap, mprotect, etc.
pub mod memory;
/// Opaque pointers used in MMTk, e.g. VMThread.
pub mod opaque_pointer;
/// Reference processing implementation.
pub mod reference_processor;

// These modules are pub(crate). They should only be used in MMTk core.

/// An analysis framework for collecting data and profiling in GC.
#[cfg(feature = "analysis")]
pub(crate) mod analysis;
/// Logging edges to check duplicated edges in GC.
#[cfg(feature = "extreme_assertions")]
pub(crate) mod edge_logger;
/// Finalization implementation.
pub(crate) mod finalizable_processor;
/// Forwarding word in object copying.
pub(crate) mod forwarding_word;
/// Access to per-object metadata (in GC byte or in side metadata).
pub(crate) mod gc_byte;
/// Access to per-object metadata with policy-specific configuration.
pub(crate) mod header_byte;
/// Heap implementation, including page resource, mmapper, etc.
pub(crate) mod heap;
/// Logger initialization
pub(crate) mod logger;
/// Various malloc implementations (conditionally compiled by features)
pub(crate) mod malloc;
/// MMTk command line options.
pub(crate) mod options;
/// Sanity checker for GC.
#[cfg(feature = "sanity")]
pub(crate) mod sanity;
/// Side metadata implementation.
pub(crate) mod side_metadata;
/// Utils for collecting statistics.
pub(crate) mod statistics;
/// Test utilities.
#[cfg(test)]
pub(crate) mod test_util;
/// A treadmill implementation.
pub(crate) mod treadmill;

// These modules are private. They are only used by other util modules.

mod generic_freelist;
mod int_array_freelist;
mod raw_memory_freelist;
// TODO: This is not used. Probably we can remoev this.
mod synchronized_counter;

pub use self::address::Address;
pub use self::address::ObjectReference;
pub use self::opaque_pointer::*;
pub use self::reference_processor::ReferenceProcessor;
pub use self::synchronized_counter::SynchronizedCounter;
