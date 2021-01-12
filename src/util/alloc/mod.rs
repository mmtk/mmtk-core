pub mod allocator;
pub mod allocators;
mod bumpallocator;
mod free_list_allocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;
pub mod tcmalloc;
pub mod hoard;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
pub use self::free_list_allocator::FreeListAllocator;
#[cfg(feature = "largeobjectspace")]
pub use self::large_object_allocator::LargeObjectAllocator;
