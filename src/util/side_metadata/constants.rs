use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::Address;

#[cfg(target_pointer_width = "32")]
pub(crate) const SIDE_METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
pub(crate) const SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0600_0000_0000) };

/// This constant represents the worst-case ratio of source data size to global side metadata.
/// A value of 2 means the space required for global side metadata must be less than 1/4th of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(crate) const GLOBAL_SIDE_METADATA_WORST_CASE_RATIO_LOG: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(crate) const GLOBAL_SIDE_METADATA_WORST_CASE_RATIO_LOG: usize = 2;

/// This constant represents the worst-case ratio of source data size to global+local side metadata.
/// A value of 1 means the space required for global+local side metadata must be less than 1/2nd of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(crate) const SIDE_METADATA_WORST_CASE_RATIO_LOG: usize = 2;
#[cfg(target_pointer_width = "64")]
pub(crate) const SIDE_METADATA_WORST_CASE_RATIO_LOG: usize = 1;

pub(crate) const SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> SIDE_METADATA_WORST_CASE_RATIO_LOG;

pub(crate) const CHUNK_MASK: usize = (1 << LOG_BYTES_IN_CHUNK) - 1;

pub(crate) const POLICY_SIDE_METADATA_OFFSET: usize =
    BYTES_IN_CHUNK >> GLOBAL_SIDE_METADATA_WORST_CASE_RATIO_LOG;
