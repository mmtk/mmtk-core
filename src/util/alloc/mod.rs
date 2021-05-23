///! Various allocators implementation.

/// The allocator trait and allocation-related functions.
pub(crate) mod allocator;
pub use allocator::fill_alignment_gap;
pub use allocator::AllocationError;
pub use allocator::Allocator;

/// Functions to ensure an object reference for an allocation has valid metadata.
mod object_ref_guard;

/// A list of all the allocators, embedded in Mutator
pub(crate) mod allocators;
pub use allocators::AllocatorSelector;

/// Bump pointer allocator
mod bumpallocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;
pub mod malloc_allocator;
pub mod mimalloc;

/// Large object allocator
mod large_object_allocator;
pub use large_object_allocator::LargeObjectAllocator;

/// An alloactor backed by malloc
mod malloc_allocator;
pub use malloc_allocator::MallocAllocator;

/// Immix allocator
pub mod immix_allocator;
pub use self::immix_allocator::ImmixAllocator;

/// Mark compact allocator (actually a bump pointer allocator with an extra heade word)
mod markcompact_allocator;
pub use markcompact_allocator::MarkCompactAllocator;

/// Embedded metadata pages
mod free_list_allocator;
pub use free_list_allocator::FreeListAllocator;

pub(crate) mod embedded_meta_data;


pub use crate::policy::mallocspace::metadata::is_alloced_by_malloc;

