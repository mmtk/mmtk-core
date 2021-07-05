use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, LOG_BYTES_IN_CHUNK};
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata;
#[cfg(target_pointer_width = "64")]
use crate::util::metadata::side_metadata::metadata_address_range_size;
#[cfg(target_pointer_width = "32")]
use crate::util::metadata::side_metadata::metadata_bytes_per_chunk;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::GLOBAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::metadata::side_metadata::LOCAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::metadata::store_metadata;
use crate::util::metadata::MetadataSpec;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::{constants, conversions, metadata};
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::{AtomicU32, Ordering};

// We use a pattern for the current mmap state of the active chunk metadata which is similar
// to the forwarding word wherein we have three states:
//   1. NOT_MAPPED: This is the initial state where the active chunk metadata has not been mmapped
//   2. MAP_IN_PROGRESS: A mutator thread has successfully atomically acquired the lock for the
//        active chunk metadata and is now in the process of mmapping its space
//   3. MAPPED: The aforementioned mutator has successfully mmapped the active chunk metadata space
const NOT_MAPPED: u32 = 0b00;
const MAP_IN_PROGRESS: u32 = 0b10;
const MAPPED: u32 = 0b11;

static FIRST_CHUNK: AtomicU32 = AtomicU32::new(NOT_MAPPED);

lazy_static! {
    pub(super) static ref CHUNK_METADATA: SideMetadataContext = SideMetadataContext {
        global: metadata::extract_side_metadata(&[MetadataSpec::OnSide(
            ACTIVE_CHUNK_METADATA_SPEC
        )]),
        local: vec![],
    };
}

/// Metadata spec for the active chunk byte
///
/// This metadata is mapped eagerly (as opposed to lazily like the others),
/// hence a separate `SideMetadata` instance is required.
///
/// This is a global side metadata spec even though it is used only by MallocSpace as
/// we require its space to be contiguous and mapped only once. Otherwise we risk
/// overwriting the previous mapping.
pub(crate) const ACTIVE_CHUNK_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 3,
    log_min_obj_size: LOG_BYTES_IN_CHUNK as usize,
};

/// This is the metadata spec for the alloc-bit.
///
/// An alloc-bit is required per min-object-size aligned address, rather than per object, and can only exist as side metadata.
///
/// The other metadata used by MallocSpace is mark-bit, which is per-object and can be kept in object header if the VM allows it.
/// Thus, mark-bit is vm-dependant and is part of each VM's ObjectModel.
///
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: false,
    offset: if cfg!(target_pointer_width = "64") {
        LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
    } else {
        0
    },
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

/// Metadata spec for the active page byte
///
/// We use a byte instead of a bit to avoid synchronization costs, i.e. to avoid
/// the case where two threads try to update different bits in the same byte at
/// the same time
#[cfg(target_pointer_width = "64")]
pub(crate) const ACTIVE_PAGE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: false,
    offset: ALLOC_SIDE_METADATA_SPEC.offset
        + metadata_address_range_size(&ALLOC_SIDE_METADATA_SPEC),
    log_num_of_bits: 3,
    log_min_obj_size: constants::LOG_BYTES_IN_PAGE as usize,
};

#[cfg(target_pointer_width = "32")]
pub(crate) const ACTIVE_PAGE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: false,
    offset: ALLOC_SIDE_METADATA_SPEC.offset
        + metadata_bytes_per_chunk(
            ALLOC_SIDE_METADATA_SPEC.log_min_obj_size,
            ALLOC_SIDE_METADATA_SPEC.log_num_of_bits,
        ),
    log_num_of_bits: 3,
    log_min_obj_size: constants::LOG_BYTES_IN_PAGE as usize,
};

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    FIRST_CHUNK.load(Ordering::Relaxed) == MAPPED && is_chunk_marked(chunk_start)
}

fn map_chunk_mark_space(chunk_start: Address) {
    // We eagerly map 16Gb worth of space for the chunk mark bytes on 64-bits
    #[cfg(target_pointer_width = "64")]
    let start = chunk_start.saturating_sub(2048 * BYTES_IN_CHUNK);
    #[cfg(target_pointer_width = "64")]
    let size = 4096 * BYTES_IN_CHUNK;

    // We eagerly map 2Gb (i.e. half the address space) worth of space for the chunk mark bytes on 32-bits
    #[cfg(target_pointer_width = "32")]
    let start = chunk_start.saturating_sub(256 * BYTES_IN_CHUNK);
    #[cfg(target_pointer_width = "32")]
    let size = 512 * BYTES_IN_CHUNK;

    info!(
        "chunk_start = {} mapping space for {} -> {}",
        chunk_start,
        start,
        chunk_start + (size / 2)
    );

    if CHUNK_METADATA.try_map_metadata_space(start, size).is_err() {
        panic!("failed to mmap meta memory");
    }
}

pub fn map_meta_space_for_chunk(metadata: &SideMetadataContext, chunk_start: Address) {
    // XXX: is there a better way to do this?
    #[allow(clippy::collapsible_if)]
    if FIRST_CHUNK.load(Ordering::SeqCst) == NOT_MAPPED {
        if FIRST_CHUNK.compare_exchange(
            NOT_MAPPED,
            MAP_IN_PROGRESS,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) == Ok(NOT_MAPPED)
        {
            map_chunk_mark_space(chunk_start);
            FIRST_CHUNK.store(MAPPED, Ordering::SeqCst);
        }
    }

    while FIRST_CHUNK.load(Ordering::Relaxed) != MAPPED {}

    if is_chunk_marked(chunk_start) {
        return;
    }

    set_chunk_mark(chunk_start);
    let mmap_metadata_result = metadata.try_map_metadata_space(chunk_start, BYTES_IN_CHUNK);
    trace!("set chunk mark bit for {}", chunk_start);
    debug_assert!(
        mmap_metadata_result.is_ok(),
        "mmap sidemetadata failed for chunk_start ({})",
        chunk_start
    );
}

// Check if a given object was allocated by malloc
pub fn is_alloced_by_malloc(object: ObjectReference) -> bool {
    is_meta_space_mapped(object.to_address()) && is_alloced(object)
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

#[allow(unused)]
pub fn is_alloced_object(address: Address) -> bool {
    side_metadata::load_atomic(ALLOC_SIDE_METADATA_SPEC, address, Ordering::SeqCst) == 1
}

#[allow(unused)]
pub unsafe fn is_alloced_object_unsafe(address: Address) -> bool {
    side_metadata::load(ALLOC_SIDE_METADATA_SPEC, address) == 1
}

#[allow(unused)]
pub fn is_marked<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) -> bool {
    load_metadata::<VM>(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        None,
        ordering,
    ) == 1
}

#[allow(unused)]
pub(super) fn is_page_marked(page_addr: Address) -> bool {
    side_metadata::load_atomic(ACTIVE_PAGE_METADATA_SPEC, page_addr, Ordering::SeqCst) == 1
}

#[allow(unused)]
pub(super) unsafe fn is_page_marked_unsafe(page_addr: Address) -> bool {
    side_metadata::load(ACTIVE_PAGE_METADATA_SPEC, page_addr) == 1
}

#[allow(unused)]
pub fn is_chunk_marked(chunk_start: Address) -> bool {
    side_metadata::load_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, Ordering::SeqCst) == 1
}

#[allow(unused)]
pub unsafe fn is_chunk_marked_unsafe(chunk_start: Address) -> bool {
    side_metadata::load(ACTIVE_CHUNK_METADATA_SPEC, chunk_start) == 1
}

#[allow(unused)]
pub fn set_alloc_bit(object: ObjectReference) {
    side_metadata::store_atomic(
        ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        1,
        Ordering::SeqCst,
    );
}

#[allow(unused)]
pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) {
    store_metadata::<VM>(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        1,
        None,
        ordering,
    );
}

#[allow(unused)]
pub(super) fn set_page_mark(page_addr: Address) {
    side_metadata::store_atomic(ACTIVE_PAGE_METADATA_SPEC, page_addr, 1, Ordering::SeqCst);
}

#[allow(unused)]
pub(super) unsafe fn set_page_mark_unsafe(page_addr: Address) {
    side_metadata::store(ACTIVE_PAGE_METADATA_SPEC, page_addr, 1);
}

#[allow(unused)]
pub(super) fn set_chunk_mark(chunk_start: Address) {
    side_metadata::store_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 1, Ordering::SeqCst);
}

#[allow(unused)]
pub(super) unsafe fn set_chunk_mark_unsafe(chunk_start: Address) {
    side_metadata::store(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 1);
}

#[allow(unused)]
pub fn unset_alloc_bit(object: ObjectReference) {
    side_metadata::store_atomic(
        ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        0,
        Ordering::SeqCst,
    );
}

#[allow(unused)]
pub unsafe fn unset_alloc_bit_unsafe(object: ObjectReference) {
    side_metadata::store(ALLOC_SIDE_METADATA_SPEC, object.to_address(), 0);
}

#[allow(unused)]
pub fn unset_mark_bit<VM: VMBinding>(object: ObjectReference, ordering: Option<Ordering>) {
    store_metadata::<VM>(
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        0,
        None,
        ordering,
    );
}

#[allow(unused)]
pub(super) fn unset_page_mark(page_addr: Address) {
    side_metadata::store_atomic(ACTIVE_PAGE_METADATA_SPEC, page_addr, 0, Ordering::SeqCst);
}

#[allow(unused)]
pub(super) unsafe fn unset_page_mark_unsafe(page_addr: Address) {
    side_metadata::store(ACTIVE_PAGE_METADATA_SPEC, page_addr, 0);
}

#[allow(unused)]
pub(super) fn unset_chunk_mark(chunk_start: Address) {
    side_metadata::store_atomic(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 0, Ordering::SeqCst);
}

#[allow(unused)]
pub(super) unsafe fn unset_chunk_mark_unsafe(chunk_start: Address) {
    side_metadata::store(ACTIVE_CHUNK_METADATA_SPEC, chunk_start, 0);
}

/// Load u128 bits of side metadata
///
/// # Safety
/// unsafe as it can segfault if one tries to read outside the bounds of the mapped side metadata
pub(super) unsafe fn load128(metadata_spec: SideMetadataSpec, data_addr: Address) -> u128 {
    let meta_addr = side_metadata::address_to_meta_address(metadata_spec, data_addr);
    if cfg!(debug_assertions) {
        side_metadata::ensure_metadata_is_mapped(metadata_spec, data_addr);
    }

    meta_addr.load::<u128>() as u128
}
