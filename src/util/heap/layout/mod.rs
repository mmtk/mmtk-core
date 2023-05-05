pub mod heap_parameters;
pub mod vm_layout_constants;

mod mmapper;
pub use self::mmapper::Mmapper;
mod byte_map_mmapper;
#[cfg(target_pointer_width = "64")]
mod fragmented_mapper;

mod map;
pub use self::map::VMMap;
mod map32;
#[cfg(target_pointer_width = "64")]
mod map64;

#[cfg(target_pointer_width = "32")]
pub fn create_vm_map() -> Box<dyn VMMap> {
    Box::new(map32::Map32::new())
}

#[cfg(target_pointer_width = "64")]
pub fn create_vm_map() -> Box<dyn VMMap> {
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

use crate::util::Address;
use std::ops::Range;

/// The heap range between HEAP_START and HEAP_END
/// Heap range include the availble range, but may include some address ranges
/// that we count as part of the heap but we do not allocate into, such as
/// VM spaces. However, currently, heap range is the same as available range.
pub const fn heap_range() -> Range<Address> {
    vm_layout_constants::HEAP_START..vm_layout_constants::HEAP_END
}

/// The avialable heap range between AVAILABLE_START and AVAILABLE_END.
/// Available range is what MMTk may allocate into.
pub const fn available_range() -> Range<Address> {
    vm_layout_constants::AVAILABLE_START..vm_layout_constants::AVAILABLE_END
}
