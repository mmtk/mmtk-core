pub mod heap_parameters;
#[macro_use]
pub mod vm_layout_constants;
pub mod mmapper;
pub use self::mmapper::Mmapper;
#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
mod byte_map_mmapper;
#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub use self::byte_map_mmapper::ByteMapMmapper;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
mod fragmented_mapper;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub use self::fragmented_mapper::FragmentedMapper;
pub mod heap_layout;
pub mod map;
#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub mod map32;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub mod map64;
