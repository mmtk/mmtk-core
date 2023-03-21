pub mod heap_parameters;
pub mod vm_layout_constants;

mod mmapper;
pub use self::mmapper::Mmapper;
mod byte_map_mmapper;
#[cfg(target_pointer_width = "64")]
mod fragmented_mapper;

mod map;
pub use self::map::Map;
mod map32;
#[cfg(target_pointer_width = "64")]
mod map64;

#[cfg(target_pointer_width = "32")]
pub fn create_vm_map() -> Box<dyn Map> {
    Box::new(map32::Map32::new())
}

#[cfg(target_pointer_width = "64")]
pub fn create_vm_map() -> Box<dyn Map> {
    // TODO: Map32 for compressed pointers
    Box::new(map64::Map64::new())
}

#[cfg(target_pointer_width = "32")]
pub fn create_mmapper() -> Box<dyn Mmapper> {
    Box::new(byte_map_mmapper::ByteMapMmapper::new())
}

#[cfg(target_pointer_width = "64")]
pub fn create_mmapper() -> Box<dyn Mmapper> {
    // TODO: ByteMapMmapper for 39-bit or less virtual space
    Box::new(fragmented_mapper::FragmentedMapper::new())
}
