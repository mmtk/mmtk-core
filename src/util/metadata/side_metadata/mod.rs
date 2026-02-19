//! This module provides an implementation of side table metadata.
// For convenience, this module is public and the bindings may create and use side metadata for their purpose.

mod layout;
pub(crate) mod helpers;
#[cfg(target_pointer_width = "32")]
mod helpers_32;

mod global;
pub(crate) mod ranges;
mod sanity;
mod side_metadata_tests;
pub(crate) mod spec_defs;

pub use layout::*;
pub use global::*;

use crate::vm::ObjectModel;
use crate::vm::VMBinding;

/// Initialize side metadata runtime state and reserve the side metadata address range.
pub fn initialize_side_metadata<VM: VMBinding>() {
    let vm_side_metadata_specs = super::extract_side_metadata(&[
        *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
        *VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
        *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
        *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        #[cfg(feature = "object_pinning")]
        *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC,
        *VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC,
    ]);
    set_vm_side_metadata_specs(&vm_side_metadata_specs);
    initialize_side_metadata_base();
}

// Re-export helper functions. Allow unused imports in case there is no function that can be re-exported.
#[allow(unused_imports)]
pub(crate) use helpers::*;
#[cfg(target_pointer_width = "32")]
#[allow(unused_imports)]
pub(crate) use helpers_32::*;
pub(crate) use sanity::SideMetadataSanity;
