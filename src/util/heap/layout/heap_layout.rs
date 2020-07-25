#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
use crate::util::heap::layout::map32::Map32;
#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
use crate::util::heap::layout::ByteMapMmapper;

#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
use crate::util::heap::layout::map64::Map64;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
use crate::util::heap::layout::FragmentedMapper;

#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub type VMMap = Map32;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub type VMMap = Map64;

#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub type Mmapper = ByteMapMmapper;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub type Mmapper = FragmentedMapper;
