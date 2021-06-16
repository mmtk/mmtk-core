pub(crate) mod allocator;
pub use allocator::fill_alignment_gap;
pub use allocator::Allocator;

pub(crate) mod allocators;
pub use allocators::AllocatorSelector;

mod bumpallocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;
pub mod malloc_allocator;
pub mod mimalloc;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
pub use self::large_object_allocator::LargeObjectAllocator;
pub use self::malloc_allocator::MallocAllocator;

mod malloc_allocator;
pub use malloc_allocator::MallocAllocator;

mod free_list_allocator;
pub use free_list_allocator::FreeListAllocator;

pub(crate) mod dump_linear_scan;
pub(crate) mod embedded_meta_data;
pub(crate) mod linear_scan;
