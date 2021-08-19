use crate::util::alloc_bit;
use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataOffset;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::LOCAL_SIDE_METADATA_BASE_OFFSET;
use crate::util::metadata::store_metadata;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::{constants, conversions};
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

lazy_static! {
    pub(super) static ref CHUNK_METADATA: SideMetadataContext = SideMetadataContext {
        global: vec![ACTIVE_CHUNK_METADATA_SPEC],
        local: vec![],
    };

    // lock to synchronize the mapping of side metadata for a newly allocated chunk by malloc
    static ref CHUNK_MAP_LOCK: Mutex<()> = Mutex::new(());
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
pub(crate) const ACTIVE_CHUNK_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: SideMetadataOffset::layout_after(&crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC),
    log_num_of_bits: 3,
    log_min_obj_size: LOG_BYTES_IN_CHUNK as usize,
};

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
pub(crate) const ACTIVE_PAGE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: false,
    offset: LOCAL_SIDE_METADATA_BASE_OFFSET,
    log_num_of_bits: 3,
    log_min_obj_size: constants::LOG_BYTES_IN_PAGE as usize,
};

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    is_chunk_mapped(chunk_start) && is_chunk_marked(chunk_start)
}

// Eagerly map the active chunk metadata surrounding `chunk_start`
fn map_active_chunk_metadata(chunk_start: Address) {
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

    if CHUNK_METADATA.try_map_metadata_space(start, size).is_err() {
        panic!("failed to mmap meta memory");
    }
}

// We map the active chunk metadata (if not previously mapped), as well as the alloc bit metadata
// and active page metadata here
pub fn map_meta_space_for_chunk(metadata: &SideMetadataContext, chunk_start: Address) {
    // In order to prevent race conditions, we synchronize on the lock first and then
    // check if we need to map the active chunk metadata for `chunk_start`
    let _lock = CHUNK_MAP_LOCK.lock().unwrap();

    // Check if the chunk bit metadata is mapped. If it is not mapped, map it.
    // Note that the chunk bit metadata is global. It may have been mapped because other policy mapped it.
    if !is_chunk_mapped(chunk_start) {
        map_active_chunk_metadata(chunk_start);
    }

    // If we have set the chunk bit, return. This is needed just in case another thread has done this before
    // we can acquire the lock.
    if is_chunk_marked(chunk_start) {
        return;
    }

    // Attempt to map the local metadata for the policy.
    // Note that this might fail. For example, we have marked a chunk as active but later we freed all
    // the objects in it, and unset its chunk bit. However, we do not free its metadata. So for the chunk,
    // its chunk bit is mapped, but not marked, and all its local metadata is also mapped.
    let mmap_metadata_result = metadata.try_map_metadata_space(chunk_start, BYTES_IN_CHUNK);
    debug_assert!(
        mmap_metadata_result.is_ok(),
        "mmap sidemetadata failed for chunk_start ({})",
        chunk_start
    );

    // Set the chunk mark at the end. So if we have chunk mark set, we know we have mapped side metadata
    // for the chunk.
    trace!("set chunk mark bit for {}", chunk_start);
    set_chunk_mark(chunk_start);
}

// Check if a given object was allocated by malloc
pub fn is_alloced_by_malloc(object: ObjectReference) -> bool {
    is_meta_space_mapped(object.to_address()) && is_alloced(object)
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

pub fn is_alloced_object(address: Address) -> bool {
    alloc_bit::is_alloced_object(address)
}

pub unsafe fn is_alloced_object_unsafe(address: Address) -> bool {
    alloc_bit::is_alloced_object_unsafe(address)
}

pub fn is_marked<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) -> bool {
    load_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        None,
        ordering,
    ) == 1
}

#[allow(unused)]
pub(super) fn is_page_marked(page_addr: Address) -> bool {
    side_metadata::load_atomic(&ACTIVE_PAGE_METADATA_SPEC, page_addr, Ordering::SeqCst) == 1
}

#[allow(unused)]
pub(super) unsafe fn is_page_marked_unsafe(page_addr: Address) -> bool {
    side_metadata::load(&ACTIVE_PAGE_METADATA_SPEC, page_addr) == 1
}

pub fn is_chunk_mapped(chunk_start: Address) -> bool {
    side_metadata::address_to_meta_address(&ACTIVE_CHUNK_METADATA_SPEC, chunk_start).is_mapped()
}

pub fn is_chunk_marked(chunk_start: Address) -> bool {
    side_metadata::load_atomic(&ACTIVE_CHUNK_METADATA_SPEC, chunk_start, Ordering::SeqCst) == 1
}

pub unsafe fn is_chunk_marked_unsafe(chunk_start: Address) -> bool {
    side_metadata::load(&ACTIVE_CHUNK_METADATA_SPEC, chunk_start) == 1
}

pub fn set_alloc_bit(object: ObjectReference) {
    alloc_bit::set_alloc_bit(object);
}

pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) {
    store_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        1,
        None,
        ordering,
    );
}

#[allow(unused)]
pub fn unset_alloc_bit(object: ObjectReference) {
    alloc_bit::unset_alloc_bit(object);
}

pub(super) fn set_page_mark(page_addr: Address) {
    side_metadata::store_atomic(&ACTIVE_PAGE_METADATA_SPEC, page_addr, 1, Ordering::SeqCst);
}

pub(super) fn set_chunk_mark(chunk_start: Address) {
    side_metadata::store_atomic(
        &ACTIVE_CHUNK_METADATA_SPEC,
        chunk_start,
        1,
        Ordering::SeqCst,
    );
}

pub unsafe fn unset_alloc_bit_unsafe(object: ObjectReference) {
    alloc_bit::unset_alloc_bit_unsafe(object);
}

#[allow(unused)]
pub fn unset_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) {
    store_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        0,
        None,
        ordering,
    );
}

pub(super) unsafe fn unset_page_mark_unsafe(page_addr: Address) {
    side_metadata::store(&ACTIVE_PAGE_METADATA_SPEC, page_addr, 0);
}

pub(super) unsafe fn unset_chunk_mark_unsafe(chunk_start: Address) {
    side_metadata::store(&ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 0);
}

/// Load u128 bits of side metadata
///
/// # Safety
/// unsafe as it can segfault if one tries to read outside the bounds of the mapped side metadata
pub(super) unsafe fn load128(metadata_spec: &SideMetadataSpec, data_addr: Address) -> u128 {
    let meta_addr = side_metadata::address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        side_metadata::ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    meta_addr.load::<u128>() as u128
}
