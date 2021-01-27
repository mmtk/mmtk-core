use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::side_metadata::SideMetadata;
use crate::util::side_metadata::SideMetadataID;
use crate::util::Address;
use crate::util::ObjectReference;
use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;
use std::sync::RwLock;
use atomic::Ordering;
use conversions::chunk_align_down;

lazy_static! {
    pub static ref MAPPED_CHUNKS: RwLock<HashSet<Address>> = RwLock::default();
}

pub static mut HEAP_SIZE: usize = 0;
pub static HEAP_USED: AtomicUsize = AtomicUsize::new(0);
pub static mut ALLOCATION_METADATA_ID: SideMetadataID = SideMetadataID::new();
pub static mut MARKING_METADATA_ID: SideMetadataID = SideMetadataID::new();

// pub struct Malloc;

// unsafe impl GlobalAlloc for Malloc {
//     unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
//         calloc(layout.align(), layout.size()) as *mut u8
//     }

//     unsafe fn dealloc(&self, ptr: *mut u8, _layout: std::alloc::Layout) {
//         free(ptr as *mut c_void)
//     }
// }
// #[global_allocator]
// static GLOBAL: Malloc = Malloc;

pub fn heap_full() -> bool {
    unsafe { HEAP_USED.load(Ordering::SeqCst) >= HEAP_SIZE }
}

pub fn meta_space_mapped(address: Address) -> bool {
    let chunk_start = chunk_align_down(address);
    MAPPED_CHUNKS.read().unwrap().contains(&chunk_start)
}

pub unsafe fn map_meta_space_for_chunk(chunk_start: Address) {
    SideMetadata::map_meta_space(chunk_start, BYTES_IN_CHUNK, ALLOCATION_METADATA_ID);
    SideMetadata::map_meta_space(chunk_start, BYTES_IN_CHUNK, MARKING_METADATA_ID);
    MAPPED_CHUNKS.write().unwrap().insert(chunk_start);
}

// Check if a given object was allocated by malloc
pub fn is_malloced(object: ObjectReference) -> bool {
    let address = object.to_address();
    unsafe { meta_space_mapped(address) && SideMetadata::load_atomic(ALLOCATION_METADATA_ID, address) == 1 }
}

// check the corresponding bit in the metadata table
pub fn is_marked(object: ObjectReference) -> bool {
    let address = object.to_address();
    debug_assert!(meta_space_mapped(address));
    unsafe { SideMetadata::load_atomic(MARKING_METADATA_ID, address) == 1 }
}

pub fn set_alloc_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        SideMetadata::store_atomic(ALLOCATION_METADATA_ID, address, 1);
    }
}

pub fn set_mark_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        SideMetadata::store_atomic(MARKING_METADATA_ID, address, 1);
    }
}

pub fn unset_alloc_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        SideMetadata::store_atomic(ALLOCATION_METADATA_ID, address, 0);
    }
}

pub fn unset_mark_bit(address: Address) {
    debug_assert!(meta_space_mapped(address));
    unsafe {
        SideMetadata::store_atomic(MARKING_METADATA_ID, address, 0);
    }
}
