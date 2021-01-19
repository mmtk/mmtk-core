pub mod heap_parameters;
#[macro_use]
pub mod vm_layout_constants;
pub mod mmapper;
pub use self::mmapper::Mmapper;
#[cfg(target_pointer_width = "32")]
mod byte_map_mmapper;
#[cfg(target_pointer_width = "32")]
pub use self::byte_map_mmapper::ByteMapMmapper;
#[cfg(target_pointer_width = "64")]
mod fragmented_mapper;
#[cfg(target_pointer_width = "64")]
pub use self::fragmented_mapper::FragmentedMapper;
pub mod heap_layout;
pub mod map;
#[cfg(target_pointer_width = "32")]
pub mod map32;
#[cfg(target_pointer_width = "64")]
pub mod map64;
