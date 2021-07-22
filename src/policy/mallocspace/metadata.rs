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
use crate::util::alloc_bit;

use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    pub static ref ACTIVE_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();
}

// /// This is the metadata spec for the alloc-bit.
// ///
// /// An alloc-bit is required per min-object-size aligned address , rather than per object, and can only exist as side metadata.
// ///
// /// The other metadata used by MallocSpace is mark-bit, which is per-object and can be kept in object header if the VM allows it.
// /// Thus, mark-bit is vm-dependant and is part of each VM's ObjectModel.
// ///
// #[cfg(target_pointer_width = "32")]
// pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
//     is_global: false,
//     offset: 0,
//     log_num_of_bits: 0,
//     log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
// };

// #[cfg(target_pointer_width = "64")]
// pub(crate) const ALLOC_SIDE_METADATA_SPEC: SideMetadataSpec = SideMetadataSpec {
//     is_global: false,
//     offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
//     log_num_of_bits: 0,
//     log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
// };

pub fn is_meta_space_mapped(address: Address) -> bool {
    let chunk_start = conversions::chunk_align_down(address);
    ACTIVE_CHUNKS.read().unwrap().contains(&chunk_start)
}

pub fn map_meta_space_for_chunk(metadata: &SideMetadataContext, chunk_start: Address) {
    let mut active_chunks = ACTIVE_CHUNKS.write().unwrap();
    if active_chunks.contains(&chunk_start) {
        return;
    }
    active_chunks.insert(chunk_start);
    alloc_bit::map_meta_space_for_chunk(metadata, chunk_start);
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

pub fn is_marked<VM: VMBinding>(object: ObjectReference) -> bool {
    load_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        None,
        Some(Ordering::SeqCst),
    ) == 1
}

pub fn set_alloc_bit(object: ObjectReference) {
    alloc_bit::set_alloc_bit(object);
}

pub fn set_mark_bit<VM: VMBinding>(object: ObjectReference) {
    store_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        1,
        None,
        Some(Ordering::SeqCst),
    );
}

pub fn unset_alloc_bit(object: ObjectReference) {
    alloc_bit::unset_alloc_bit(object);
}

pub fn unset_mark_bit<VM: VMBinding>(object: ObjectReference) {
    store_metadata::<VM>(
        &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        object,
        0,
        None,
        Some(Ordering::SeqCst),
    );
}
