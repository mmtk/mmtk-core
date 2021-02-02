use crate::util::Address;

#[cfg(target_pointer_width = "32")]
pub(super) const SIDE_METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
pub(super) const SIDE_METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0600_0000_0000) };

#[cfg(target_pointer_width = "32")]
pub(super) const GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 4;
#[cfg(target_pointer_width = "64")]
pub(super) const GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 4;

#[cfg(target_pointer_width = "32")]
pub(super) const POLICY_SIDE_METADATA_WORST_CASE_RATIO: usize = 4;
#[cfg(target_pointer_width = "64")]
pub(super) const POLICY_SIDE_METADATA_WORST_CASE_RATIO: usize = 4;
