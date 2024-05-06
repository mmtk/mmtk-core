//! This module provides an implementation of side table metadata.
// For convenience, this module is public and the bindings may create and use side metadata for their purpose.

mod constants;
mod helpers;
#[cfg(target_pointer_width = "32")]
mod helpers_32;

mod global;
mod sanity;
mod side_metadata_tests;
pub(crate) mod spec_defs;

pub use constants::*;
pub use global::*;

// Re-export helper functions. Allow unused imports in case there is no function that can be re-exported.
#[allow(unused_imports)]
pub(crate) use helpers::*;
#[cfg(target_pointer_width = "32")]
#[allow(unused_imports)]
pub(crate) use helpers_32::*;
pub(crate) use sanity::SideMetadataSanity;
