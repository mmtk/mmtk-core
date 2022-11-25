use crate::util::alloc_bit;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

lazy_static! {
    pub(super) static ref CHUNK_METADATA: SideMetadataContext = SideMetadataContext {
        global: vec![ACTIVE_CHUNK_METADATA_SPEC],
        local: vec![],
    };

    /// Lock to synchronize the mapping of side metadata for a newly allocated chunk by malloc
    static ref CHUNK_MAP_LOCK: Mutex<()> = Mutex::new(());
    /// Maximum metadata address for the ACTIVE_CHUNK_METADATA_SPEC which is used to check bounds
    pub static ref MAX_METADATA_ADDRESS: Address = ACTIVE_CHUNK_METADATA_SPEC.upper_bound_address_for_contiguous();
}

/// Metadata spec for the active chunk byte
///
/// The active chunk metadata is used to track what chunks have been allocated by `malloc()`
/// which is out of our control. We use this metadata later to generate sweep tasks for only
/// the chunks which have live objects in them.
///
/// This metadata is mapped eagerly (as opposed to lazily like the others),
/// hence a separate `SideMetadata` instance is required.
///
/// This is a global side metadata spec even though it is used only by MallocSpace as
/// we require its space to be contiguous and mapped only once. Otherwise we risk
/// overwriting the previous mapping.
pub(crate) const ACTIVE_CHUNK_METADATA_SPEC: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::MS_ACTIVE_CHUNK;

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

/// Check if metadata is mapped for a range [addr, addr + size). Metadata is mapped per chunk,
/// we will go through all the chunks for [address, address + size), and check if they are mapped.
/// If any of the chunks is not mapped, return false. Otherwise return true.
pub fn is_meta_space_mapped(address: Address, size: usize) -> bool {
    let mut chunk = conversions::chunk_align_down(address);
    while chunk < address + size {
        if !is_meta_space_mapped_for_address(chunk) {
            return false;
        }
        chunk += BYTES_IN_CHUNK;
    }
    true
}

/// Check if metadata is mapped for a given address. We check if the active chunk metadata is mapped,
/// and if the active chunk bit is marked as well. If the chunk is mapped and marked, we consider the
/// metadata for the chunk is properly mapped.
fn is_meta_space_mapped_for_address(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    is_chunk_mapped(chunk_start) && is_chunk_marked(chunk_start)
}

/// Eagerly map the active chunk metadata surrounding `chunk_start`
fn map_active_chunk_metadata(chunk_start: Address) {
    debug_assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
    // We eagerly map 16Gb worth of space for the chunk mark bytes on 64-bits
    // We require saturating subtractions in order to not overflow the chunk_start by
    // accident when subtracting if we have been allocated a very low base address by `malloc()`
    #[cfg(target_pointer_width = "64")]
    let start = chunk_start.saturating_sub(2048 * BYTES_IN_CHUNK);
    #[cfg(target_pointer_width = "64")]
    let size = 4096 * BYTES_IN_CHUNK;

    // We eagerly map 2Gb (i.e. half the address space) worth of space for the chunk mark bytes on 32-bits
    #[cfg(target_pointer_width = "32")]
    let start = chunk_start.saturating_sub(256 * BYTES_IN_CHUNK);
    #[cfg(target_pointer_width = "32")]
    let size = 512 * BYTES_IN_CHUNK;

    debug!(
        "chunk_start = {} mapping space for {} -> {}",
        chunk_start,
        start,
        chunk_start + (size / 2)
    );

    assert!(
        CHUNK_METADATA.try_map_metadata_space(start, size).is_ok(),
        "failed to mmap meta memory"
    );
}

/// We map the active chunk metadata (if not previously mapped), as well as the alloc bit metadata
/// and active page metadata here. Note that if [addr, addr + size) crosses multiple chunks, we
/// will map for each chunk.
pub fn map_meta_space(metadata: &SideMetadataContext, addr: Address, size: usize) {
    // In order to prevent race conditions, we synchronize on the lock first and then
    // check if we need to map the active chunk metadata for `chunk_start`
    let _lock = CHUNK_MAP_LOCK.lock().unwrap();

    let map_metadata_space_for_chunk = |start: Address| {
        debug_assert!(start.is_aligned_to(BYTES_IN_CHUNK));
        // Check if the chunk bit metadata is mapped. If it is not mapped, map it.
        // Note that the chunk bit metadata is global. It may have been mapped because other policy mapped it.
        if !is_chunk_mapped(start) {
            map_active_chunk_metadata(start);
        }

        // If we have set the chunk bit, return. This is needed just in case another thread has done this before
        // we can acquire the lock.
        if is_chunk_marked(start) {
            return;
        }

        // Attempt to map the local metadata for the policy.
        // Note that this might fail. For example, we have marked a chunk as active but later we freed all
        // the objects in it, and unset its chunk bit. However, we do not free its metadata. So for the chunk,
        // its chunk bit is mapped, but not marked, and all its local metadata is also mapped.
        let mmap_metadata_result = metadata.try_map_metadata_space(start, BYTES_IN_CHUNK);
        debug_assert!(
            mmap_metadata_result.is_ok(),
            "mmap sidemetadata failed for chunk_start ({})",
            start
        );

        // Set the chunk mark at the end. So if we have chunk mark set, we know we have mapped side metadata
        // for the chunk.
        trace!("set chunk mark bit for {}", start);
        set_chunk_mark(start);
    };

    // Go through each chunk, and map for them.
    let mut chunk = conversions::chunk_align_down(addr);
    while chunk < addr + size {
        map_metadata_space_for_chunk(chunk);
        chunk += BYTES_IN_CHUNK;
    }
}

/// Check if a given object was allocated by malloc
pub fn is_alloced_by_malloc<VM: VMBinding>(object: ObjectReference) -> bool {
    is_meta_space_mapped_for_address(object.to_address::<VM>())
        && alloc_bit::is_alloced::<VM>(object)
}

/// Check if there is an object allocated by malloc at the address.
///
/// This function doesn't check if `addr` is aligned.
/// If not, it will try to load the alloc bit for the address rounded down to the metadata's granularity.
#[cfg(feature = "is_mmtk_object")]
pub fn has_object_alloced_by_malloc<VM: VMBinding>(addr: Address) -> Option<ObjectReference> {
    if !is_meta_space_mapped_for_address(addr) {
        return None;
    }
    alloc_bit::is_alloced_object::<VM>(addr)
}

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

pub fn is_chunk_mapped(chunk_start: Address) -> bool {
    // Since `address_to_meta_address` will translate a data address to a metadata address without caring
    // if it goes across metadata boundaries, we have to check if we have accidentally gone over the bounds
    // of the active chunk metadata spec before we check if the metadata has been mapped or not
    let meta_address =
        side_metadata::address_to_meta_address(&ACTIVE_CHUNK_METADATA_SPEC, chunk_start);
    if meta_address < *MAX_METADATA_ADDRESS {
        meta_address.is_mapped()
    } else {
        false
    }
}

pub fn is_chunk_marked(chunk_start: Address) -> bool {
    ACTIVE_CHUNK_METADATA_SPEC.load_atomic::<u8>(chunk_start, Ordering::SeqCst) == 1
}

pub unsafe fn is_chunk_marked_unsafe(chunk_start: Address) -> bool {
    ACTIVE_CHUNK_METADATA_SPEC.load::<u8>(chunk_start) == 1
}

pub fn set_alloc_bit<VM: VMBinding>(object: ObjectReference) {
    alloc_bit::set_alloc_bit::<VM>(object);
}

pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Ordering) {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(object, 1, None, ordering);
}

#[allow(unused)]
pub fn unset_alloc_bit<VM: VMBinding>(object: ObjectReference) {
    alloc_bit::unset_alloc_bit::<VM>(object);
}

#[allow(unused)]
pub(super) fn set_page_mark(page_addr: Address) {
    ACTIVE_PAGE_METADATA_SPEC.store_atomic::<u8>(page_addr, 1, Ordering::SeqCst);
}

pub(super) fn set_chunk_mark(chunk_start: Address) {
    ACTIVE_CHUNK_METADATA_SPEC.store_atomic::<u8>(chunk_start, 1, Ordering::SeqCst);
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

pub unsafe fn unset_alloc_bit_unsafe<VM: VMBinding>(object: ObjectReference) {
    alloc_bit::unset_alloc_bit_unsafe::<VM>(object);
}

#[allow(unused)]
pub unsafe fn unset_mark_bit<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store::<VM, u8>(object, 0, None);
}

#[allow(unused)]
pub(super) unsafe fn unset_page_mark_unsafe(page_addr: Address) {
    ACTIVE_PAGE_METADATA_SPEC.store::<u8>(page_addr, 0)
}

pub(super) unsafe fn unset_chunk_mark_unsafe(chunk_start: Address) {
    ACTIVE_CHUNK_METADATA_SPEC.store::<u8>(chunk_start, 0)
}

/// Load u128 bits of side metadata
///
/// # Safety
/// unsafe as it can segfault if one tries to read outside the bounds of the mapped side metadata
pub(super) unsafe fn load128(metadata_spec: &SideMetadataSpec, data_addr: Address) -> u128 {
    let meta_addr = side_metadata::address_to_meta_address(metadata_spec, data_addr);

    #[cfg(debug_assertions)]
    metadata_spec.assert_metadata_mapped(data_addr);

    meta_addr.load::<u128>() as u128
}
