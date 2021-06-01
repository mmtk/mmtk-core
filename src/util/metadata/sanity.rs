#[cfg(feature = "extreme_assertions")]
use super::MetadataSpec;
use super::{side_metadata, MetadataContext};
#[cfg(feature = "extreme_assertions")]
use crate::util::Address;

pub struct MetadataSanity {
    pub side_metadata_sanity: side_metadata::SideMetadataSanity,
}

impl MetadataSanity {
    pub fn new() -> MetadataSanity {
        MetadataSanity {
            side_metadata_sanity: side_metadata::SideMetadataSanity::new(),
        }
    }

    pub(crate) fn verify_metadata_context(
        &mut self,
        policy_name: &'static str,
        metadata_context: &MetadataContext,
    ) {
        self.side_metadata_sanity
            .verify_metadata_context(policy_name, &metadata_context.filter(false));
    }

    #[cfg(test)]
    pub fn reset(&mut self) {
        self.side_metadata_sanity.reset();
    }
}

impl Default for MetadataSanity {
    fn default() -> Self {
        Self::new()
    }
}

/// Commits a side metadata bulk zero operation.
/// Panics if the metadata spec is not valid.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to perform the bulk zeroing on
/// * `start`: the starting address of the source data
/// * `size`: size of the source data
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_bzero(metadata_spec: MetadataSpec, start: Address, size: usize) {
    if metadata_spec.is_on_side {
        side_metadata::sanity::verify_bzero(metadata_spec, start, size)
    } else {
        todo!()
    }
}

/// Ensures a side metadata load operation returns the correct side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - data address is not valid,
/// 3 - the loaded side metadata content is not equal to the correct content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the loaded content for
/// * `data_addr`: the address of the source data
/// * `actual_val`: the actual content returned by the side metadata load operation
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_load(metadata_spec: &MetadataSpec, data_addr: Address, actual_val: usize) {
    if metadata_spec.is_on_side {
        side_metadata::sanity::verify_load(metadata_spec, data_addr, actual_val)
    } else {
        todo!()
    }
}

/// Commits a side metadata store operation.
/// Panics if:
/// 1 - the loaded side metadata content is not equal to the correct content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to commit the store operation for
/// * `data_addr`: the address of the source data
/// * `metadata`: the metadata content to store
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_store(metadata_spec: MetadataSpec, data_addr: Address, metadata: usize) {
    if metadata_spec.is_on_side {
        side_metadata::sanity::verify_store(metadata_spec, data_addr, metadata)
    } else {
        todo!()
    }
}

/// Commits a fetch and add operation and ensures it returns the correct old side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - the old side metadata content is not equal to the correct old content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the old content for
/// * `data_addr`: the address of the source data
/// * `val_to_add`: the number to be added to the old content
/// * `actual_old_val`: the actual old content returned by the side metadata fetch and add operation
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_add(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    val_to_add: usize,
    actual_old_val: usize,
) {
    if metadata_spec.is_on_side {
        side_metadata::sanity::verify_add(metadata_spec, data_addr, val_to_add, actual_old_val)
    } else {
        todo!()
    }
}

/// Commits a fetch and sub operation and ensures it returns the correct old side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - the old side metadata content is not equal to the correct old content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the old content for
/// * `data_addr`: the address of the source data
/// * `val_to_sub`: the number to be subtracted from the old content
/// * `actual_old_val`: the actual old content returned by the side metadata fetch and sub operation
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_sub(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    val_to_sub: usize,
    actual_old_val: usize,
) {
    if metadata_spec.is_on_side {
        side_metadata::sanity::verify_sub(metadata_spec, data_addr, val_to_sub, actual_old_val)
    } else {
        todo!()
    }
}
