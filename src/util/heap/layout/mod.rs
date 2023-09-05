pub mod heap_parameters;
pub mod vm_layout;

mod mmapper;
pub use self::mmapper::Mmapper;
mod byte_map_mmapper;
#[cfg(target_pointer_width = "64")]
mod fragmented_mapper;

mod map;
pub use self::map::VMMap;
use self::vm_layout::vm_layout;
mod map32;
#[cfg(target_pointer_width = "64")]
mod map64;

#[cfg(target_pointer_width = "32")]
pub fn create_vm_map() -> Box<dyn VMMap + Send + Sync> {
    Box::new(map32::Map32::new())
}

#[cfg(target_pointer_width = "64")]
pub fn create_vm_map() -> Box<dyn VMMap + Send + Sync> {
    if !vm_layout().force_use_contiguous_spaces {
        Box::new(map32::Map32::new())
    } else {
        Box::new(map64::Map64::new())
    }
}

#[cfg(target_pointer_width = "32")]
pub fn create_mmapper() -> Box<dyn Mmapper + Send + Sync> {
    Box::new(byte_map_mmapper::ByteMapMmapper::new())
}

#[cfg(target_pointer_width = "64")]
pub fn create_mmapper() -> Box<dyn Mmapper + Send + Sync> {
    // TODO: ByteMapMmapper for 39-bit or less virtual space
    Box::new(fragmented_mapper::FragmentedMapper::new())
}

use crate::util::Address;
use std::ops::Range;

/// The heap range between HEAP_START and HEAP_END
/// Heap range include the availble range, but may include some address ranges
/// that we count as part of the heap but we do not allocate into, such as
/// VM spaces. However, currently, heap range is the same as available range.
pub fn heap_range() -> Range<Address> {
    vm_layout().heap_start..vm_layout().heap_end
}

/// The avialable heap range between AVAILABLE_START and AVAILABLE_END.
/// Available range is what MMTk may allocate into.
pub fn available_range() -> Range<Address> {
    vm_layout().available_start()..vm_layout().available_end()
}
