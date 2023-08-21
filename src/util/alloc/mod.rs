//! Various allocators implementation.

/// The allocator trait and allocation-related functions.
pub(crate) mod allocator;
pub use allocator::fill_alignment_gap;
pub use allocator::AllocationError;
pub use allocator::Allocator;

/// A list of all the allocators, embedded in Mutator
pub(crate) mod allocators;
pub use allocators::AllocatorInfo;
pub use allocators::AllocatorSelector;

/// Bump pointer allocator
mod bumpallocator;
pub use bumpallocator::BumpAllocator;
pub use bumpallocator::BumpPointer;

use crate::util::Address;
use crate::vm::VMBinding;
pub fn bump_alloc_check<VM: VMBinding>(allocator: &mut BumpAllocator<VM>, size: usize, align: usize, offset: usize) -> Address {
    allocator.alloc(size, align, offset)
}

mod large_object_allocator;
pub use large_object_allocator::LargeObjectAllocator;

/// An alloactor backed by malloc
mod malloc_allocator;
pub use malloc_allocator::MallocAllocator;

/// Immix allocator
pub mod immix_allocator;
pub use self::immix_allocator::ImmixAllocator;

// Free list allocator based on Mimalloc
pub mod free_list_allocator;
pub use free_list_allocator::FreeListAllocator;

/// Mark compact allocator (actually a bump pointer allocator with an extra heade word)
mod markcompact_allocator;
pub use markcompact_allocator::MarkCompactAllocator;

/// Embedded metadata pages
pub(crate) mod embedded_meta_data;
