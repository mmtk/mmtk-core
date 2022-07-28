use crate::mmtk::SFT_MAP;
use crate::policy::sft_map::SFTMap;
use crate::util::Address;

/// The region size (in bytes) of the `ALLOC_BIT` side metadata.
/// The VM can use this to check if an object is properly aligned.
pub const ALLOC_BIT_REGION_SIZE: usize =
    1usize << crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC.log_bytes_in_region;

#[inline(always)]
pub(crate) fn is_mmtk_object(addr: Address) -> bool {
    SFT_MAP.get(addr).is_mmtk_object(addr)
}
