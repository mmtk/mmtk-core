//! This module includes functions to make sure the following invariant always holds: for each object we allocate (`[cell, cell + bytes)`), the metadata for
//! the object reference (`object_ref`) is always in the range of the allocated memory. Given that we always initialize metadata based on chunks,
//! we simply need to make sure that `object_ref` is in the same chunk as `[cell, cell + bytes)`. In other words, we avoid
//! allocating an address for which the object reference may be in another chunk.
//!
//! Note that where an ObjectReference points to is defined by a binding. We only have this problem if an object reference may point
//! to an address that is outside our allocated memory (`object_ref >= cell + bytes`). We ask a binding to specify
//! `ObjectModel::OBJECT_REF_OFFSET_BEYOND_CELL` if their object reference may point to an address outside the allocated
//! memory. `ObjectModel::OBJECT_REF_OFFSET_BEYOND_CELL` should be the max of `object_ref - cell`.
//!
//! There are various ways we deal with this.
//! * For allocators that have a thread local buffer, we can adjust the buffer limit to make sure that the last object allocated in the
//!   buffer won't cross chunks.
//! * For allocators that allocate large objects, if the object size is larger than `OBJECT_REF_OFFSET_BEYOND_CELL`, it is guaranteed that
//!   the object reference is within the allocated memory.
//! * For other allocators, we can check if the allocation result violates this invariant.

use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, CHUNK_MASK};
use crate::util::Address;
use crate::vm::VMBinding;

/// Adjust limit for thread local buffer to make sure that we will not allocate objects whose object reference may
/// be in another chunk.
pub fn adjust_thread_local_buffer_limit<VM: VMBinding>(limit: Address) -> Address {
    // We only need to adjust limit when the binding tells us that
    // object ref may point outside the allocated memory and when limit is at chunk boundary
    if let Some(offset) = VM::OBJECT_REF_OFFSET_BEYOND_CELL {
        if limit.is_aligned_to(BYTES_IN_CHUNK) {
            debug_assert!(limit.as_usize() > offset);
            // We simply not use the last few bytes. This is a rare case anyway (expect less than 1% of slowpath allocation goes here).
            // It should be possible for us to check if we can use the last few bytes to finish an allocation request when we 'exhaust'
            // thread local buffer. But probably it won't give us much benefit and it complicates our allocation code.
            return limit - offset;
        }
    }

    limit
}

/// Assert that the object reference should always inside the allocation cell
#[cfg(debug_assertions)]
pub fn assert_object_ref_in_cell<VM: VMBinding>(size: usize) {
    if VM::OBJECT_REF_OFFSET_BEYOND_CELL.is_none() {
        return;
    }

    // If the object ref offset is smaller than size, it is always inside the allocation cell.
    debug_assert!(
        size > VM::OBJECT_REF_OFFSET_BEYOND_CELL.unwrap(),
        "Allocating objects of size {} may cross chunk (OBJECT_REF_OFFSET_BEYOND_CELL = {})",
        size,
        VM::OBJECT_REF_OFFSET_BEYOND_CELL.unwrap()
    );
}

/// Check if the object reference for this allocation may cross and fall into the next chunk.
pub fn object_ref_may_cross_chunk<VM: VMBinding>(addr: Address) -> bool {
    if VM::OBJECT_REF_OFFSET_BEYOND_CELL.is_none() {
        return false;
    }

    (addr & CHUNK_MASK) + VM::OBJECT_REF_OFFSET_BEYOND_CELL.unwrap() >= BYTES_IN_CHUNK
}
