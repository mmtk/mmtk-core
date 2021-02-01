pub mod allocator;
pub mod allocators;
mod bumpallocator;
pub mod dump_linear_scan;
pub mod embedded_meta_data;
pub mod large_object_allocator;
pub mod linear_scan;
#[cfg(feature = "marksweep")]
pub mod malloc_allocator;

pub use self::allocator::Allocator;
pub use self::bumpallocator::BumpAllocator;
#[cfg(feature = "largeobjectspace")]
pub use self::large_object_allocator::LargeObjectAllocator;
#[cfg(feature = "marksweep")]
pub use self::malloc_allocator::MallocAllocator;

#[cfg(feature = "marksweep")]
pub use crate::plan::marksweep::metadata::is_malloced;
