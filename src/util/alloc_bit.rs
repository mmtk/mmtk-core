use atomic::Ordering;

use crate::util::constants;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSpec;
#[cfg(target_pointer_width = "64")]
use crate::util::metadata::side_metadata::LOCAL_SIDE_METADATA_BASE_ADDRESS;
use crate::util::metadata::store_metadata;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::{ObjectModel, VMBinding};


/// This is the metadata spec for the alloc-bit.
///
/// An alloc-bit is required per min-object-size aligned address , rather than per object, and can only exist as side metadata.
///
#[cfg(target_pointer_width = "32")]
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: 0,
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

#[cfg(target_pointer_width = "64")]
pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
    is_global: true,
    offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 0,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};


pub fn map_meta_space_for_chunk(metadata: &SideMetadataContext, chunk_start: Address) {
    let mmap_metadata_result = metadata.try_map_metadata_space(chunk_start, BYTES_IN_CHUNK);
    debug_assert!(
        mmap_metadata_result.is_ok(),
        "mmap sidemetadata failed for chunk_start ({})",
        chunk_start
    );
}


pub fn set_alloc_bit(object: ObjectReference) {
    side_metadata::store_atomic(
        &ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        1,
        Ordering::SeqCst,
    );
}

pub fn unset_addr_alloc_bit(address: Address) {
    side_metadata::store_atomic(
        &ALLOC_SIDE_METADATA_SPEC,
        address,
        0,
        Ordering::SeqCst,
    );
}

pub fn unset_alloc_bit(object: ObjectReference) {
    side_metadata::store_atomic(
        &ALLOC_SIDE_METADATA_SPEC,
        object.to_address(),
        0,
        Ordering::SeqCst,
    );
}

pub fn is_alloced(object: ObjectReference) -> bool {
    is_alloced_object(object.to_address())
}

pub fn is_alloced_object(address: Address) -> bool {
    side_metadata::load_atomic(&ALLOC_SIDE_METADATA_SPEC, address, Ordering::SeqCst) == 1
}

pub fn bzero_alloc_bit(start: Address, size: usize) {
    side_metadata::bzero_metadata(&ALLOC_SIDE_METADATA_SPEC, start, size);
}



