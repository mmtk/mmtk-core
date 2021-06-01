// TODO - check module visibility
mod global;
mod header_metadata;
mod sanity;
mod side_metadata;

pub(crate) use global::*;
pub(crate) use sanity::*;
pub(crate) use side_metadata::GLOBAL_SIDE_METADATA_BASE_ADDRESS;
#[cfg(any(target_pointer_width = "64", test))]
pub(crate) use side_metadata::LOCAL_SIDE_METADATA_BASE_ADDRESS;
