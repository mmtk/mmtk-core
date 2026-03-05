#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::VMLayout;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
#[cfg(target_pointer_width = "64")]
use crate::util::metadata::side_metadata::side_metadata_offset_after;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::os::{MmapAnnotation, MmapStrategy};
use crate::util::Address;
use crate::util::{constants::LOG_BYTES_IN_PAGE, conversions::raw_align_up};
use crate::MMAPPER;
use std::sync::OnceLock;

/// The compile-time base offset for global side metadata layout. We treat offsets as relative
/// (starting from zero) and add the runtime base address when computing actual addresses.
pub(crate) const GLOBAL_SIDE_METADATA_BASE_OFFSET: usize = 0;

/// The run-time base address for side metadata. This is initialized at startup by mmapping necessary memory address for side metadata,
/// and should be used as the base when computing actual side metadata addresses.
/// We use OnceLock to ensure it is only initialized once. To eliminate the cost of accessing OnceLock after initialization, we can use get().unwrap_unchecked().
static SIDE_METADATA_BASE_ADDRESS: OnceLock<Address> = OnceLock::new();

/// The upper bound for VM side metadata layout. We need to list all the VM side metadata specs, compute the upper bound, and store it here.
static VM_SIDE_METADATA_UPPER_BOUND_OFFSET: OnceLock<usize> = OnceLock::new();

// The following steps are needed before using side metadata.
// The functions are intended as 'pub(super)'. We expect most people to use [`crate::util::metadata::side_metadata::initialize_side_metadata()`] instead, which calls these functions internally.

// Step 1: Call `set_vm-side_metadata_specs()` to register VM side metadata layout. This is needed for the startup reservation to cover VM side metadata.

/// Record VM side metadata layout so startup reservation can cover VM specs.
/// This must be called before `initialize_side_metadata_base()`.
pub(super) fn set_vm_side_metadata_specs(specs: &[SideMetadataSpec]) {
    let mut upper_bound = 0usize;
    for spec in specs {
        if spec.is_absolute_offset() {
            upper_bound = upper_bound.max(spec.upper_bound_offset());
        }
    }
    let _ = VM_SIDE_METADATA_UPPER_BOUND_OFFSET.set(upper_bound);
    debug!(
        "Registered VM side metadata layout: {} specs, upper_bound={}",
        specs.len(),
        upper_bound
    );
}

// Step 2: Call `initialize_side_metadata_base()` to reserve address space for side metadata.

/// Initialize the side metadata base address by reserving address space with quarantine mmap.
pub(super) fn initialize_side_metadata_base() {
    SIDE_METADATA_BASE_ADDRESS.get_or_init(|| {
        #[cfg(target_pointer_width = "64")]
        {
            let core_end = super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();
            let vm_end = *VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().unwrap();
            info!(
                "Initializing side metadata base: vm_specs_registered={} core_end={} vm_end={}",
                VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().is_some(),
                core_end,
                unsafe { Address::from_usize(vm_end) }
            );
            if VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().is_none() {
                warn!(
                    "Initializing side metadata base before VM side metadata layout was registered"
                );
                let bt = std::backtrace::Backtrace::capture();
                debug!("backtrace for early side metadata base initialization:\n{bt}");
            }
        }
        let total_bytes = side_metadata_reserved_bytes();
        let pages = total_bytes >> LOG_BYTES_IN_PAGE;
        let anno = MmapAnnotation::SideMeta {
            space: "side-metadata",
            meta: "all",
        };
        info!(
            "Quarantine side metadata range: total_bytes=0x{:x}, pages=0x{:x}, granularity=0x{:x}",
            total_bytes,
            pages,
            MMAPPER.granularity()
        );
        let base = MMAPPER
            .quarantine_address_range_anywhere(pages, MmapStrategy::SIDE_METADATA, &anno)
            .unwrap_or_else(|e| panic!("failed to quarantine side metadata address range: {e}"));
        info!(
            "Side metadata base initialized at {} (range: {} - {})",
            base,
            base,
            base + total_bytes
        );
        base
    });
}

// With the above functions called, side metadata is initialized, and the following functions can be used to query side metadata layout and addresses.

/// Get the runtime side metadata base address.
pub fn global_side_metadata_base_address() -> Address {
    #[cfg(debug_assertions)]
    {
        #[cfg(not(any(test, feature = "test_private")))]
        {
            assert!(
                VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().is_some(),
                "global_side_metadata_base_address() called before VM side metadata layout was registered"
            );
        }

        assert!(
            SIDE_METADATA_BASE_ADDRESS.get().is_some(),
            "global_side_metadata_base_address() called before side metadata base was initialized"
        );
    }

    unsafe { *SIDE_METADATA_BASE_ADDRESS.get().unwrap_unchecked() }
}

// Local side metadata start address. This is derived from the end of global side metadata.
pub(crate) fn local_side_metadata_base_address() -> Address {
    global_side_metadata_base_address() + global_side_metadata_bytes()
}

/// Total side metadata bytes that should be reserved at startup (independent of runtime base, without alignment).
fn total_side_metadata_bytes() -> usize {
    let vm_end = *VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().unwrap();
    #[cfg(target_pointer_width = "64")]
    {
        let core_end = super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();
        debug!(
            "total_side_metadata_bytes(): core_end={} vm_end={} (registered={})",
            core_end,
            vm_end,
            VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().is_some()
        );
        core_end.max(vm_end)
    }
    #[cfg(target_pointer_width = "32")]
    {
        let local_bytes =
            1usize << (VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);
        return global_side_metadata_bytes().max(vm_end) + local_bytes;
    }
}

/// Total side metadata bytes that should be reserved at startup (independent of runtime base, with alignment).
pub(crate) fn side_metadata_reserved_bytes() -> usize {
    raw_align_up(total_side_metadata_bytes(), MMAPPER.granularity())
}

/// Base address of VO bit, public to VM bindings which may need to use this.
#[cfg(target_pointer_width = "64")]
pub fn vo_bit_side_metadata_addr() -> Address {
    crate::util::metadata::vo_bit::vo_bit_side_metadata_addr()
}

/// The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal global side metadata.
pub fn global_side_metadata_vm_base_address() -> Address {
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_address_for_contiguous()
}

/// Total global side metadata bytes (independent of the runtime base address).
pub(crate) fn global_side_metadata_bytes() -> usize {
    let end = super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_offset();
    end.max(*VM_SIDE_METADATA_UPPER_BOUND_OFFSET.get().unwrap())
}

// constants

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

#[cfg(target_pointer_width = "32")]
pub(super) const LOCAL_SIDE_METADATA_PER_CHUNK: usize =
    BYTES_IN_CHUNK >> LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;

/// The base offset for the global side metadata available to VM bindings.
pub const GLOBAL_SIDE_METADATA_VM_BASE_OFFSET: usize =
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
pub const LOCAL_SIDE_METADATA_VM_BASE_OFFSET: usize =
    super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();

#[cfg(target_pointer_width = "32")]
pub(super) const LOCAL_SIDE_METADATA_BASE_OFFSET_FOR_LAYOUT: usize = 0;
#[cfg(target_pointer_width = "64")]
pub(super) const LOCAL_SIDE_METADATA_BASE_OFFSET_FOR_LAYOUT: usize =
    side_metadata_offset_after(&super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC);
