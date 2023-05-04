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

/// Return true if the given address in our heap range
pub fn address_in_heap(addr: Address) -> bool {
    addr >= vm_layout_constants::HEAP_START && addr < vm_layout_constants::HEAP_END
}

/// Return true if the given address in our available heap range (where we manage and allocate)
pub fn address_in_avialable_range(addr: Address) -> bool {
    addr >= vm_layout_constants::AVAILABLE_START && addr < vm_layout_constants::AVAILABLE_END
}

/// Return true if the given range overlaps with our heap range
pub fn range_overlaps_heap(addr: Address, size: usize) -> bool {
    !(addr >= vm_layout_constants::HEAP_END || addr + size <= vm_layout_constants::HEAP_START)
}

/// Return true if the given range overlaps with our available heap range
pub fn range_overlaps_available_range(addr: Address, size: usize) -> bool {
    !(addr >= vm_layout_constants::AVAILABLE_END
        || addr + size <= vm_layout_constants::AVAILABLE_START)
}

/// Return true if the given range is within our heap range
pub fn range_in_heap(addr: Address, size: usize) -> bool {
    addr >= vm_layout_constants::HEAP_START && addr + size <= vm_layout_constants::HEAP_END
}

/// Return true if the given range is within our available heap range
pub fn range_in_available_range(addr: Address, size: usize) -> bool {
    addr >= vm_layout_constants::AVAILABLE_START
        && addr + size <= vm_layout_constants::AVAILABLE_END
}
