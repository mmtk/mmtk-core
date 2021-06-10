use atomic::Ordering;

use crate::util::{
    constants::{LOG_BITS_IN_ADDRESS, LOG_MIN_OBJECT_SIZE},
    Address,
};

#[cfg(target_pointer_width = "32")]
use super::metadata_bytes_per_chunk;
use super::{
    metadata_address_range_size, MetadataSpec, GLOBAL_SIDE_METADATA_BASE_ADDRESS,
    LOCAL_SIDE_METADATA_BASE_ADDRESS,
};

/// This module includes `MetadataSpec` instances for all per_object metadata bit-sets, assuming that all of these should be allocated on side.
/// This module is used in implementing the metadata part of `ObjectModel`.

// private helper functions

#[cfg(target_pointer_width = "64")]
const fn side_metadata_size(metadata_spec: MetadataSpec) -> usize {
    metadata_address_range_size(metadata_spec)
}
#[cfg(target_pointer_width = "32")]
const fn side_metadata_size(metadata_spec: MetadataSpec) -> usize {
    if metadata_spec.is_global {
        metadata_address_range_size(metadata_spec)
    } else {
        metadata_bytes_per_chunk(metadata_spec.log_min_obj_size, metadata_spec.num_of_bits)
    }
}

// Global ones

pub const LOGGING_SIDE_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: true,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_isize(),
    num_of_bits: 1,
    log_min_obj_size: 3,
};

// PolicySpecific ones

// Assume the default location of forwarding pointer is the header word
pub const FORWARDING_POINTER_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: false,
    is_global: false,
    offset: 0,
    num_of_bits: 1 << LOG_BITS_IN_ADDRESS,
    log_min_obj_size: LOG_MIN_OBJECT_SIZE as usize,
};

pub const FORWARDING_BITS_SIDE_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_isize(),
    num_of_bits: 2,
    log_min_obj_size: LOG_MIN_OBJECT_SIZE as usize,
};

pub const MARKING_SIDE_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: FORWARDING_BITS_SIDE_METADATA_SPEC.offset
        + side_metadata_size(FORWARDING_BITS_SIDE_METADATA_SPEC) as isize,
    num_of_bits: 1,
    log_min_obj_size: LOG_MIN_OBJECT_SIZE as usize,
};

pub const LOS_SIDE_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: MARKING_SIDE_METADATA_SPEC.offset
        + side_metadata_size(MARKING_SIDE_METADATA_SPEC) as isize,
    num_of_bits: 2,
    log_min_obj_size: LOG_MIN_OBJECT_SIZE as usize,
};

pub const UNLOGGED_SIDE_METADATA_SPEC: MetadataSpec = MetadataSpec {
    is_side_metadata: true,
    is_global: false,
    offset: LOS_SIDE_METADATA_SPEC.offset + side_metadata_size(LOS_SIDE_METADATA_SPEC) as isize,
    num_of_bits: 1,
    log_min_obj_size: LOG_MIN_OBJECT_SIZE as usize,
};

pub const LAST_GLOBAL_SIDE_METADATA_OFFSET: usize =
    GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + side_metadata_size(LOGGING_SIDE_METADATA_SPEC);

pub const LAST_LOCAL_SIDE_METADATA_OFFSET: usize =
    UNLOGGED_SIDE_METADATA_SPEC.offset as usize + side_metadata_size(UNLOGGED_SIDE_METADATA_SPEC);

pub fn default_store(
    spec: MetadataSpec,
    data_addr: Address,
    val: usize,
    atomic_ordering: Option<Ordering>,
) {
    if let Some(order) = atomic_ordering {
        super::store_atomic(spec, data_addr, val, order)
    } else {
        unsafe { super::store(spec, data_addr, val) }
    }
}

pub fn default_load(
    spec: MetadataSpec,
    data_addr: Address,
    atomic_ordering: Option<Ordering>,
) -> usize {
    if let Some(order) = atomic_ordering {
        super::load_atomic(spec, data_addr, order)
    } else {
        unsafe { super::load(spec, data_addr) }
    }
}
