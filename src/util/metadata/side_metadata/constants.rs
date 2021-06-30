use crate::util::heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::Address;

// This is currently not used in 32-bits targets, but ultimately it is required in 32-bits global side metadata. So, instead of guarding with target_pointer_width, I allow unused_imports for now.
#[allow(unused_imports)]
use super::metadata_address_range_size;
#[cfg(target_pointer_width = "32")]
use super::metadata_bytes_per_chunk;

#[cfg(target_pointer_width = "32")]
pub(crate) const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
pub(crate) const GLOBAL_SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0600_0000_0000usize) };

/// This constant represents the worst-case ratio of source data size to global side metadata.
/// A value of 2 means the space required for global side metadata must be less than 1/4th of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

/// This constant represents the worst-case ratio of source data size to global+local side metadata.
/// A value of 1 means the space required for global+local side metadata must be less than 1/2nd of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

const LOG_MAX_GLOBAL_SIDE_METADATA_SIZE: usize =
    LOG_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
// TODO - we should check this limit somewhere
// pub(crate) const LOG_MAX_LOCAL_SIDE_METADATA_SIZE: usize =
//     1 << (LOG_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);

pub(crate) const LOCAL_SIDE_METADATA_BASE_ADDRESS: Address = unsafe {
    Address::from_usize(
        GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
            + (1usize << LOG_MAX_GLOBAL_SIDE_METADATA_SIZE),
    )
};

#[cfg(target_pointer_width = "32")]
pub(super) const CHUNK_MASK: usize = (1 << LOG_BYTES_IN_CHUNK) - 1;

#[cfg(target_pointer_width = "32")]
pub(super) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;

/// The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal global side metadata.
pub const GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS: Address = GLOBAL_SIDE_METADATA_BASE_ADDRESS;

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
#[cfg(target_pointer_width = "64")]
pub const LOCAL_SIDE_METADATA_VM_BASE_ADDRESS: Address = LOCAL_SIDE_METADATA_BASE_ADDRESS.add(
    metadata_address_range_size(&crate::policy::mallocspace::metadata::ACTIVE_CHUNK_METADATA_SPEC)
        + metadata_address_range_size(
            &crate::policy::mallocspace::metadata::ALLOC_SIDE_METADATA_SPEC,
        )
        + metadata_address_range_size(
            &crate::policy::mallocspace::metadata::ACTIVE_PAGE_METADATA_SPEC,
        ),
);

#[cfg(target_pointer_width = "32")]
pub const LOCAL_SIDE_METADATA_VM_BASE_ADDRESS: Address = LOCAL_SIDE_METADATA_BASE_ADDRESS.add(
    // The + 1 comes from the ACTIVE_CHUNK_METADATA_SPEC
    1 + metadata_bytes_per_chunk(
        crate::policy::mallocspace::metadata::ALLOC_SIDE_METADATA_SPEC.log_min_obj_size,
        crate::policy::mallocspace::metadata::ALLOC_SIDE_METADATA_SPEC.log_num_of_bits,
    ) + metadata_bytes_per_chunk(
        crate::policy::mallocspace::metadata::ACTIVE_PAGE_METADATA_SPEC.log_min_obj_size,
        crate::policy::mallocspace::metadata::ACTIVE_PAGE_METADATA_SPEC.log_num_of_bits,
    ),
);
