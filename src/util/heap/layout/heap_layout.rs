#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::map32::Map32;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::ByteMapMmapper;

#[cfg(target_pointer_width = "64")]
use crate::util::heap::layout::map64::Map64;
#[cfg(target_pointer_width = "64")]
use crate::util::heap::layout::FragmentedMapper;

#[cfg(target_pointer_width = "32")]
pub type VMMap = Map32;
#[cfg(target_pointer_width = "64")]
pub type VMMap = Map64;

#[cfg(target_pointer_width = "32")]
pub type Mmapper = ByteMapMmapper;
#[cfg(target_pointer_width = "64")]
pub type Mmapper = FragmentedMapper;
