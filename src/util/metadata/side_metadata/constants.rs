#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::VMLayout;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
use crate::util::os::{MmapAnnotation, MmapStrategy};
use crate::util::metadata::side_metadata::{SideMetadataOffset, SideMetadataSpec};
use crate::util::Address;
use crate::util::{constants::LOG_BYTES_IN_PAGE, conversions::raw_align_up};
use crate::MMAPPER;
use std::sync::Once;

// The compile-time base offset for global side metadata layout. We treat offsets as relative
// (starting from zero) and add the runtime base address when computing actual addresses.
pub(crate) const GLOBAL_SIDE_METADATA_BASE_OFFSET: SideMetadataOffset =
    SideMetadataOffset::addr(Address::ZERO);

static mut SIDE_METADATA_BASE_ADDRESS: Address = Address::ZERO;
static BASE_INIT: Once = Once::new();

/// Set the runtime side metadata base address. The address can only be assigned once.
pub fn set_side_metadata_base_address(base: Address) {
    BASE_INIT.call_once(|| unsafe {
        SIDE_METADATA_BASE_ADDRESS = base;
    });
    let existing = unsafe { SIDE_METADATA_BASE_ADDRESS };
    assert_eq!(
        existing, base,
        "side metadata base address already initialized ({existing}), cannot reset to {base}"
    );
}

/// Get the runtime side metadata base address.
pub fn global_side_metadata_base_address() -> Address {
    #[cfg(debug_assertions)]
    {
        // Ensure initialization happens (Once provides synchronization).
        initialize_side_metadata_base();
    }

    unsafe { SIDE_METADATA_BASE_ADDRESS }
}

fn upper_bound_address_for_contiguous_relative(spec: &SideMetadataSpec) -> Address {
    debug_assert!(spec.is_absolute_offset());
    let rel = spec.offset.addr_value();
    rel.add(super::metadata_address_range_size(spec))
}

/// Base address of VO bit, public to VM bindings which may need to use this.
#[cfg(target_pointer_width = "64")]
pub fn vo_bit_side_metadata_addr() -> Address {
    crate::util::metadata::vo_bit::vo_bit_side_metadata_addr()
}

/// This constant represents the worst-case ratio of source data size to global side metadata.
/// A value of 2 means the space required for global side metadata must be less than 1/4th of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

/// This constant represents the worst-case ratio of source data size to global+local side metadata.
/// A value of 1 means the space required for global+local side metadata must be less than 1/2nd of the source data.
/// So, a value of `n` means this ratio must be less than $2^-n$.
#[cfg(target_pointer_width = "32")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 3;
#[cfg(target_pointer_width = "64")]
pub(super) const LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO: usize = 1;

// Local side metadata start address. This is derived from the end of global side metadata.
pub(crate) fn local_side_metadata_base_address() -> Address {
    global_side_metadata_base_address() + global_side_metadata_bytes()
}

/// Total side metadata bytes that should be reserved at startup (independent of runtime base).
pub(crate) fn total_side_metadata_bytes() -> usize {
    #[cfg(target_pointer_width = "64")]
    {
        let end = upper_bound_address_for_contiguous_relative(
            &super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC,
        );
        end.get_extent(Address::ZERO)
    }
    #[cfg(target_pointer_width = "32")]
    {
        let local_bytes =
            1usize << (VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);
        return global_side_metadata_bytes() + local_bytes;
    }
}

/// Initialize the side metadata base address by reserving address space with quarantine mmap.
pub fn initialize_side_metadata_base() {
    BASE_INIT.call_once(|| {
        let total_bytes = raw_align_up(total_side_metadata_bytes(), MMAPPER.granularity());
        let pages = total_bytes >> LOG_BYTES_IN_PAGE;
        let anno = MmapAnnotation::SideMeta {
            space: "side-metadata",
            meta: "all",
        };
        let base = MMAPPER
            .quarantine_address_range_anywhere(pages, MmapStrategy::SIDE_METADATA, &anno)
            .unwrap_or_else(|e| panic!("failed to quarantine side metadata address range: {e}"));
        unsafe {
            SIDE_METADATA_BASE_ADDRESS = base;
        }
    });
}

#[cfg(target_pointer_width = "32")]
pub(super) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;

/// The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal global side metadata.
pub fn global_side_metadata_vm_base_address() -> Address {
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_address_for_contiguous()
}
/// The base offset for the global side metadata available to VM bindings.
pub const GLOBAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
pub const LOCAL_SIDE_METADATA_VM_BASE_OFFSET: SideMetadataOffset =
    super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// Total global side metadata bytes (independent of the runtime base address).
pub(crate) fn global_side_metadata_bytes() -> usize {
    let end = upper_bound_address_for_contiguous_relative(
        &super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC,
    );
    end.get_extent(Address::ZERO)
}
