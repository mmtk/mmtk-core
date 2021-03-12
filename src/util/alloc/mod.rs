pub mod allocator;
pub mod allocators;
mod bumpallocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;
pub mod malloc_allocator;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
pub use self::large_object_allocator::LargeObjectAllocator;
pub use self::malloc_allocator::MallocAllocator;

pub use crate::policy::mallocspace::metadata::is_alloced_by_malloc;
