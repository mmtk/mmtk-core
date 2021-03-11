use crate::util::heap::layout::vm_layout_constants::{
    BYTES_IN_CHUNK, HEAP_END, LOG_ADDRESS_SPACE, LOG_BYTES_IN_CHUNK,
};
use crate::util::Address;

#[cfg(target_pointer_width = "32")]
pub const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
pub const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address = HEAP_END;

/// This constant represents the worst-case ratio of source data size to global side metadata.
/// A value of 2 means the space required for global side metadata must be less than 1/4th of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(crate) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(crate) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

/// This constant represents the worst-case ratio of source data size to global+local side metadata.
/// A value of 1 means the space required for global+local side metadata must be less than 1/2nd of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(crate) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(crate) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

pub(crate) const LOG_MAX_GLOBAL_SIDE_METADATA_SIZE: usize =
    LOG_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
// TODO - we should check this limit somewhere
// pub(crate) const LOG_MAX_LOCAL_SIDE_METADATA_SIZE: usize =
//     1 << (LOG_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);

pub const LOCAL_SIDE_METADATA_BASE_ADDRESS: Address = unsafe {
    Address::from_usize(
        GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
            + (1usize << LOG_MAX_GLOBAL_SIDE_METADATA_SIZE),
    )
};

pub(crate) const CHUNK_MASK: usize = (1 << LOG_BYTES_IN_CHUNK) - 1;

pub(crate) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;
