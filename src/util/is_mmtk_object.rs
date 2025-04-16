/// The region size (in bytes) of the `VO_BIT` side metadata.
/// The VM can use this to check if an object is properly aligned.
pub const VO_BIT_REGION_SIZE: usize =
    1usize << crate::util::metadata::vo_bit::VO_BIT_SIDE_METADATA_SPEC.log_bytes_in_region;

use crate::util::{Address, ObjectReference};

pub(crate) fn check_object_reference(addr: Address) -> Option<ObjectReference> {
    use crate::mmtk::SFT_MAP;
    SFT_MAP.get_checked(addr).is_mmtk_object(addr)
}

pub(crate) fn check_internal_reference(
    addr: Address,
    max_search_bytes: usize,
) -> Option<ObjectReference> {
    use crate::mmtk::SFT_MAP;
    let ret = SFT_MAP
        .get_checked(addr)
        .find_object_from_internal_pointer(addr, max_search_bytes);
    #[cfg(debug_assertions)]
    if let Some(obj) = ret {
        let obj = check_object_reference(obj.to_raw_address());
        debug_assert_eq!(obj, ret);
    }
    ret
}
