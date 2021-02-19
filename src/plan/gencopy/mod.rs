pub(super) mod gc_works;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::GenCopy;

pub const NO_BARRIER: bool = false;
pub const FULL_NURSERY_GC: bool = true;
pub const NO_SLOW: bool = true;

pub use self::global::GENCOPY_CONSTRAINTS;

use crate::util::side_metadata::*;

const LOGGING_META: SideMetadataSpec = SideMetadataSpec {
   scope: SideMetadataScope::Global,
   offset: 0,
   log_num_of_bits: 0,
   log_min_obj_size: 3,
};