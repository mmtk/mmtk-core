use crate::util::heap::layout::ByteMapMmapper;
use crate::util::heap::layout::map32::Map32;

// FIXME: Use FragmentMmapper for 64-bit heaps
// FIXME: Use Map64 for 64-bit heaps

#[cfg(target_pointer_width = "32")]
pub type VMMap = Map32;
#[cfg(target_pointer_width = "64")]
pub type VMMap = Map32;

pub type Mmapper = ByteMapMmapper;
