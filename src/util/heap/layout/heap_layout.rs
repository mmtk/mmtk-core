use crate::util::heap::layout::map32::Map32;
use crate::util::heap::layout::map64::Map64;
use crate::util::heap::layout::ByteMapMmapper;
use crate::util::heap::layout::FragmentedMapper;

// FIXME: Use FragmentMmapper for 64-bit heaps
// FIXME: Use Map64 for 64-bit heaps

#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub type VMMap = Map32;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub type VMMap = Map64;

#[cfg(any(target_pointer_width = "32", feature = "force_32bit_heap_layout"))]
pub type Mmapper = ByteMapMmapper;
#[cfg(all(target_pointer_width = "64", not(feature = "force_32bit_heap_layout")))]
pub type Mmapper = FragmentedMapper;