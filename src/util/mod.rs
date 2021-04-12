//! Utilities used by other modules, including allocators, heap implementation, etc.

#[macro_use]
pub mod macros;
#[macro_use]
pub mod conversions;
pub mod address;
pub mod alloc;
#[cfg(feature = "analysis")]
pub mod analysis;
pub mod constants;
#[cfg(feature = "extreme_assertions")]
pub mod edge_logger;
pub mod finalizable_processor;
pub mod forwarding_word;
pub mod gc_byte;
pub mod generic_freelist;
pub mod header_byte;
pub mod heap;
pub mod int_array_freelist;
pub mod logger;
pub mod malloc;
pub mod memory;
pub mod opaque_pointer;
pub mod options;
pub mod raw_memory_freelist;
pub mod reference_processor;
#[cfg(feature = "sanity")]
pub mod sanity;
pub mod side_metadata;
pub mod statistics;
mod synchronized_counter;
pub mod treadmill;

#[cfg(test)]
pub mod test_util;

pub use self::address::Address;
pub use self::address::ObjectReference;
pub use self::opaque_pointer::OpaquePointer;
pub use self::reference_processor::ReferenceProcessor;
pub use self::synchronized_counter::SynchronizedCounter;
