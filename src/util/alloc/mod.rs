//! Various allocators implementation.

/// The allocator trait and allocation-related functions.
pub(crate) mod allocator;
pub use allocator::fill_alignment_gap;
pub use allocator::AllocationError;
pub use allocator::AllocationOptions;
pub use allocator::Allocator;

/// A list of all the allocators, embedded in Mutator
pub(crate) mod allocators;
pub use allocators::AllocatorInfo;
pub use allocators::AllocatorSelector;

/// Bump pointer allocator
mod bumpallocator;
pub use bumpallocator::BumpAllocator;
pub use bumpallocator::BumpPointer;

mod large_object_allocator;
pub use large_object_allocator::LargeObjectAllocator;

/// An alloactor backed by malloc
mod malloc_allocator;
pub use malloc_allocator::MallocAllocator;

/// Immix allocator
pub mod immix_allocator;
pub use self::immix_allocator::ImmixAllocator;

/// Free list allocator based on Mimalloc
pub mod free_list_allocator;
pub use free_list_allocator::FreeListAllocator;

/// Lisp2 allocator (actually a bump pointer allocator with an extra heade word)
mod lisp2_allocator;
pub use lisp2_allocator::Lisp2Allocator;

/// Embedded metadata pages
pub(crate) mod embedded_meta_data;
