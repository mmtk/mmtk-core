use crate::util::side_metadata::*;
use crate::util::ObjectReference;
#[cfg(not(feature = "side_gc_header"))]
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
#[cfg(not(feature = "side_gc_header"))]
use std::sync::atomic::{AtomicU8, Ordering};

use super::constants;

pub const SIDE_GC_BYTE_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::Global,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 1,
    log_min_obj_size: constants::LOG_MIN_OBJECT_SIZE as usize,
};

// TODO: we probably need to add non-atomic versions of the read and write methods
/// Return the GC byte of an object as an atomic.
///
/// MMTk requires *exactly one byte* for each object as per-object metadata.
///
/// If the client VM provides the one byte in its object headers
/// (see [trait ObjectModel](crate::vm::ObjectModel)),
/// MMTk uses that byte as the per-object metadata.
/// Otherwise, MMTk provides the metadata on its side.
///
#[inline(always)]
#[cfg(not(feature = "side_gc_header"))]
fn get_gc_byte<VM: VMBinding>(object: ObjectReference) -> &'static AtomicU8 {
    unsafe { &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET).to_ptr::<AtomicU8>() }
}

/// Atomically reads the current value of an object's GC byte.
///
/// Returns an 8-bit unsigned integer
#[inline(always)]
pub fn read_gc_byte<VM: VMBinding>(object: ObjectReference) -> u8 {
    #[cfg(not(feature = "side_gc_header"))]
    {
        get_gc_byte::<VM>(object).load(Ordering::SeqCst)
    }
    #[cfg(feature = "side_gc_header")]
    {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        load_atomic(SIDE_GC_BYTE_SPEC, object.to_address()) as u8
    }
}

/// Atomically writes a new value to the GC byte of an object
#[inline(always)]
pub fn write_gc_byte<VM: VMBinding>(object: ObjectReference, val: u8) {
    #[cfg(not(feature = "side_gc_header"))]
    {
        get_gc_byte::<VM>(object).store(val, Ordering::SeqCst);
    }
    #[cfg(feature = "side_gc_header")]
    {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        store_atomic(SIDE_GC_BYTE_SPEC, object.to_address(), val as usize);
    }
}

/// Atomically performs the compare-and-exchange operation on the GC byte of an object.
///
/// Returns `true` if the operation succeeds.
#[inline(always)]
pub fn compare_exchange_gc_byte<VM: VMBinding>(
    object: ObjectReference,
    old_val: u8,
    new_val: u8,
) -> bool {
    #[cfg(not(feature = "side_gc_header"))]
    {
        get_gc_byte::<VM>(object)
            .compare_exchange(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    #[cfg(feature = "side_gc_header")]
    {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        compare_exchange_atomic(
            SIDE_GC_BYTE_SPEC,
            object.to_address(),
            old_val as usize,
            new_val as usize,
        )
    }
}
