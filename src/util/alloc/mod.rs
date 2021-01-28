pub mod allocator;
pub mod allocators;
mod bumpallocator;
mod global_allocator;
#[cfg(feature = "mallocms")]
pub mod malloc_allocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
#[cfg(feature = "mallocms")]
pub use self::malloc_allocator::MallocAllocator;
#[cfg(feature = "largeobjectspace")]
pub use self::large_object_allocator::LargeObjectAllocator;

#[cfg(feature = "mallocms")]
pub use crate::plan::mallocms::metadata::is_malloced;