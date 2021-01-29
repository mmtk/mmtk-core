pub mod allocator;
pub mod allocators;
mod bumpallocator;
mod global_allocator;
#[cfg(feature = "marksweep")]
pub mod malloc_allocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
#[cfg(feature = "marksweep")]
pub use self::malloc_allocator::MallocAllocator;
#[cfg(feature = "largeobjectspace")]
pub use self::large_object_allocator::LargeObjectAllocator;

#[cfg(feature = "marksweep")]
pub use crate::plan::marksweep::metadata::is_malloced;