//! This module includes functions to make sure the following invariant always holds: for each object we allocate (`[cell, cell + bytes)`), the metadata for
//! the object reference (`object_ref`) is always in the range of the allocated memory. Given that we always initialize metadata based on chunks,
//! we simply need to make sure that `object_ref` is in the same chunk as `[cell, cell + bytes)`. In other words, we avoid
//! allocating an address for which the object reference may be in another chunk.
//!
//! Note that where an ObjectReference points to is defined by a binding. We only have this problem if an object reference may point
//! to an address that is outside our allocated memory (`object_ref >= cell + bytes` or `object_ref < alloc`). We ask a binding to specify
//! `ObjectModel::OBJECT_REF_OFFSET_MAYBE_OUTSIDE_ALLOCATION` if their object reference may point to an address outside the allocated
//! memory. `ObjectModel::OBJECT_REF_OFFSET_FROM_ALLOCATION` should be `object_ref - cell`.
//!
//! There are various ways we deal with this.
//! * For allocators that have a thread local buffer, we can adjust the buffer limit to make sure that the last object allocated in the
//!   buffer won't cross chunks.
//! * For allocators that allocate large objects, if the object size is larger than `OBJECT_REF_OFFSET_MAYBE_OUTSIDE_ALLOCATION`, it is guaranteed that
//!   the object reference is within the allocated memory.
//! * For other allocators, we can check if the allocation result violates this invariant.

use crate::util::heap::layout::vm_layout_constants::{BYTES_IN_CHUNK, CHUNK_MASK};
use crate::util::Address;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;

/// Adjust start and end for thread local buffer to make sure that we will not allocate objects whose object reference may
/// be in another chunk.
pub fn adjust_thread_local_buffer_range<VM: VMBinding>(
    start: Address,
    end: Address,
) -> (Address, Address) {
    // We only need to adjust start/end when the binding tells us that
    // object ref may point outside the allocated memory
    if VM::VMObjectModel::OBJECT_REF_MAYBE_OUTSIDE_ALLOCATION {
        // If the buffer starts at the chunk boundary, and the object ref is before the allocation address,
        // the object ref could possibly be in the previous chunk. We need to adjust the start.
        let adjusted_start = if start.is_aligned_to(BYTES_IN_CHUNK)
            && VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND < 0
        {
            start + (-VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND)
        } else {
            start
        };
        // If the buffer ends at the chunk boundary, and the object ref is after the allocation address,
        // the object ref could possibly be in the next chunk. We need to adjust the end.
        let adjusted_end = if end.is_aligned_to(BYTES_IN_CHUNK)
            && VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND > 0
        {
            end + (-VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND)
        } else {
            end
        };
        return (adjusted_start, adjusted_end);
    }

    (start, end)
}

/// Assert that the object reference should always inside the allocation cell. This is used when an allocator
/// does not specially treat the out-of-bound case for object reference. Instead, they just assert that the object
/// ref is not possible to be outside the allocation region. If the assertion in this method fails, that means we
/// need to consider the out-of-bound case for the allocator.
#[cfg(debug_assertions)]
pub fn assert_object_ref_in_allocation<VM: VMBinding>(size: usize) {
    // If the binding declares that their object ref won't be outside the allocation region, then we don't need to assert anything.
    if !VM::VMObjectModel::OBJECT_REF_MAYBE_OUTSIDE_ALLOCATION {
        return;
    }

    // If the object ref offset is positive, as long as the allocation size is larger than the upper bound, we will be fine.
    if VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND > 0 {
        debug_assert!(
            size as isize > VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND,
            "Allocating objects of size {} may cross chunk.",
            size
        );
    }
    // If the object ref offset is negative, this won't work -- we need to reserve some room if the allocation address is at chunk boundary.
    if VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND < 0 {
        panic!("Object reference offset is negative. It is possible the object reference is outside the allocation. We need to implement for this case");
    }
}

/// Check if the object reference for this allocation may cross and fall into the next chunk.
pub fn object_ref_may_cross_chunk<VM: VMBinding>(addr: Address) -> bool {
    if !VM::VMObjectModel::OBJECT_REF_MAYBE_OUTSIDE_ALLOCATION {
        return false;
    }

    let masked_addr = (addr & CHUNK_MASK) as isize;

    masked_addr + VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND >= BYTES_IN_CHUNK as isize
        || masked_addr + VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND < 0
}
