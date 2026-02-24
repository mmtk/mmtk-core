use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::vo_bit;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::Ordering;

/// Metadata spec for the active page byte
///
/// The active page metadata is used to accurately track the total number of pages that have
/// been reserved by `malloc()`.
///
/// We use a byte instead of a bit to avoid synchronization costs, i.e. to avoid
/// the case where two threads try to update different bits in the same byte at
/// the same time
// XXX: This metadata spec is currently unused as we need to add a performant way to calculate
// how many pages are active in this metadata spec. Explore SIMD vectorization with 8-bit integers
pub(crate) const ACTIVE_PAGE_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::MALLOC_MS_ACTIVE_PAGE;

pub(crate) const OFFSET_MALLOC_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::MS_OFFSET_MALLOC;

pub fn is_marked<VM: VMBinding>(object: ObjectReference, ordering: Ordering) -> bool {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(object, None, ordering) == 1
}

pub unsafe fn is_marked_unsafe<VM: VMBinding>(object: ObjectReference) -> bool {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load::<VM, u8>(object, None) == 1
}

/// Set the page mark from 0 to 1. Return true if we set it successfully in this call.
pub(super) fn compare_exchange_set_page_mark(page_addr: Address) -> bool {
    // The spec has 1 byte per each page. So it won't be the case that other threads may race and access other bits for the spec.
    // If the compare-exchange fails, we know the byte was set to 1 before this call.
    ACTIVE_PAGE_METADATA_SPEC
        .compare_exchange_atomic::<u8>(page_addr, 0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

#[allow(unused)]
pub(super) fn is_page_marked(page_addr: Address) -> bool {
    ACTIVE_PAGE_METADATA_SPEC.load_atomic::<u8>(page_addr, Ordering::SeqCst) == 1
}

#[allow(unused)]
pub(super) unsafe fn is_page_marked_unsafe(page_addr: Address) -> bool {
    ACTIVE_PAGE_METADATA_SPEC.load::<u8>(page_addr) == 1
}

pub fn set_vo_bit(object: ObjectReference) {
    vo_bit::set_vo_bit(object);
}

pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Ordering) {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(object, 1, None, ordering);
}

#[allow(unused)]
pub fn unset_vo_bit(object: ObjectReference) {
    vo_bit::unset_vo_bit(object);
}

#[allow(unused)]
pub(super) fn set_page_mark(page_addr: Address) {
    ACTIVE_PAGE_METADATA_SPEC.store_atomic::<u8>(page_addr, 1, Ordering::SeqCst);
}

/// Is this allocation an offset malloc? The argument address should be the allocation address (object start)
pub(super) fn is_offset_malloc(address: Address) -> bool {
    unsafe { OFFSET_MALLOC_METADATA_SPEC.load::<u8>(address) == 1 }
}

/// Set the offset bit for the allocation. The argument address should be the allocation address (object start)
pub(super) fn set_offset_malloc_bit(address: Address) {
    OFFSET_MALLOC_METADATA_SPEC.store_atomic::<u8>(address, 1, Ordering::SeqCst);
}

/// Unset the offset bit for the allocation. The argument address should be the allocation address (object start)
pub(super) unsafe fn unset_offset_malloc_bit_unsafe(address: Address) {
    OFFSET_MALLOC_METADATA_SPEC.store::<u8>(address, 0);
}

pub unsafe fn unset_vo_bit_unsafe(object: ObjectReference) {
    vo_bit::unset_vo_bit_unsafe(object);
}

#[allow(unused)]
pub unsafe fn unset_mark_bit<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store::<VM, u8>(object, 0, None);
}

#[allow(unused)]
pub(super) unsafe fn unset_page_mark_unsafe(page_addr: Address) {
    ACTIVE_PAGE_METADATA_SPEC.store::<u8>(page_addr, 0)
}

/// Load u128 bits of side metadata
///
/// # Safety
/// unsafe as it can segfault if one tries to read outside the bounds of the mapped side metadata
pub(super) unsafe fn load128(metadata_spec: &SideMetadataSpec, data_addr: Address) -> u128 {
    let meta_addr = side_metadata::address_to_meta_address(metadata_spec, data_addr);

    #[cfg(all(debug_assertions, feature = "extreme_assertions"))]
    metadata_spec.assert_metadata_mapped(data_addr);

    meta_addr.load::<u128>()
}
