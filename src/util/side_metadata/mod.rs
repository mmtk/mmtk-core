//! A generic module to work with side metadata.
//!

mod global;
mod helpers;

pub use global::SideMetadata;
pub use global::SideMetadataID;
pub use helpers::address_to_meta_page_address;
pub use helpers::meta_page_is_mapped;
