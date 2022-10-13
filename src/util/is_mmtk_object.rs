/// The region size (in bytes) of the `VO_BIT` side metadata.
/// The VM can use this to check if an object is properly aligned.
pub const VO_BIT_REGION_SIZE: usize =
    1usize << crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC.log_bytes_in_region;
