#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::VMLayout;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::os::{MmapAnnotation, MmapStrategy};
use crate::util::Address;
use crate::util::{constants::LOG_BYTES_IN_PAGE, conversions::raw_align_up};
use crate::MMAPPER;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

// The compile-time base offset for global side metadata layout. We treat offsets as relative
// (starting from zero) and add the runtime base address when computing actual addresses.
pub(crate) const GLOBAL_SIDE_METADATA_BASE_OFFSET: usize =
    0;

static mut SIDE_METADATA_BASE_ADDRESS: Address = Address::ZERO;
static BASE_INIT: Once = Once::new();
static VM_SIDE_METADATA_LAYOUT_INIT: Once = Once::new();
static mut VM_SIDE_METADATA_UPPER_BOUND_OFFSET: Address = Address::ZERO;
static VM_SIDE_METADATA_LAYOUT_REGISTERED: AtomicBool = AtomicBool::new(false);

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
    #[cfg(all(debug_assertions, not(any(test, feature = "test_private"))))]
    {
        assert!(
            VM_SIDE_METADATA_LAYOUT_REGISTERED.load(Ordering::SeqCst),
            "global_side_metadata_base_address() called before VM side metadata layout was registered"
        );
    }

    unsafe { SIDE_METADATA_BASE_ADDRESS }
}

/// Record VM side metadata layout so startup reservation can cover VM specs.
/// This must be called before `initialize_side_metadata_base()`.
pub(crate) fn set_vm_side_metadata_specs(specs: &[SideMetadataSpec]) {
    VM_SIDE_METADATA_LAYOUT_INIT.call_once(|| {
        #[cfg(target_pointer_width = "64")]
        {
            let mut upper_bound = Address::ZERO;
            for spec in specs {
                if spec.is_absolute_offset() {
                    upper_bound = upper_bound.max(unsafe { Address::from_usize(spec.upper_bound_offset()) });
                }
            }
            unsafe {
                VM_SIDE_METADATA_UPPER_BOUND_OFFSET = upper_bound;
            }
            VM_SIDE_METADATA_LAYOUT_REGISTERED.store(true, Ordering::SeqCst);
            debug!(
                "Registered VM side metadata layout: {} specs, upper_bound={}",
                specs.len(),
                upper_bound
            );
        }
    });
}

fn upper_bound_address_for_contiguous_relative(spec: &SideMetadataSpec) -> Address {
    debug_assert!(spec.is_absolute_offset());
    let rel = unsafe { Address::from_usize(spec.offset) };
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
        let core_end = upper_bound_address_for_contiguous_relative(
            &super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC,
        );
        let vm_end = unsafe { VM_SIDE_METADATA_UPPER_BOUND_OFFSET };
        debug!(
            "total_side_metadata_bytes(): core_end={} vm_end={} (registered={})",
            core_end,
            vm_end,
            VM_SIDE_METADATA_LAYOUT_REGISTERED.load(Ordering::SeqCst)
        );
        core_end.max(vm_end).get_extent(Address::ZERO)
    }
    #[cfg(target_pointer_width = "32")]
    {
        let local_bytes =
            1usize << (VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO);
        return global_side_metadata_bytes() + local_bytes;
    }
}

pub(crate) fn side_metadata_reserved_bytes() -> usize {
    raw_align_up(total_side_metadata_bytes(), MMAPPER.granularity())
}

pub(crate) fn side_metadata_reserved_range() -> std::ops::Range<Address> {
    let base = global_side_metadata_base_address();
    let bytes = side_metadata_reserved_bytes();
    base..(base + bytes)
}

/// Initialize the side metadata base address by reserving address space with quarantine mmap.
pub(crate) fn initialize_side_metadata_base() {
    BASE_INIT.call_once(|| {
        #[cfg(target_pointer_width = "64")]
        {
            let core_end = upper_bound_address_for_contiguous_relative(
                &super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC,
            );
            let vm_end = unsafe { VM_SIDE_METADATA_UPPER_BOUND_OFFSET };
            info!(
                "Initializing side metadata base: vm_specs_registered={} core_end={} vm_end={}",
                VM_SIDE_METADATA_LAYOUT_REGISTERED.load(Ordering::SeqCst),
                core_end,
                vm_end
            );
            if !VM_SIDE_METADATA_LAYOUT_REGISTERED.load(Ordering::SeqCst) {
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
        unsafe {
            SIDE_METADATA_BASE_ADDRESS = base;
        }
        info!(
            "Side metadata base initialized at {} (range: {} - {})",
            base,
            base,
            base + total_bytes
        );
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
pub const GLOBAL_SIDE_METADATA_VM_BASE_OFFSET: usize =
    super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata.
/// VM bindings must use this to avoid overlap with core internal local side metadata.
pub const LOCAL_SIDE_METADATA_VM_BASE_OFFSET: usize =
    super::spec_defs::LAST_LOCAL_SIDE_METADATA_SPEC.upper_bound_offset();

/// Total global side metadata bytes (independent of the runtime base address).
pub(crate) fn global_side_metadata_bytes() -> usize {
    let end = upper_bound_address_for_contiguous_relative(
        &super::spec_defs::LAST_GLOBAL_SIDE_METADATA_SPEC,
    );
    end.get_extent(Address::ZERO)
}
