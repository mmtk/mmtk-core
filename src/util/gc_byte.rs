use crate::util::side_metadata::*;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicU8, Ordering};

use super::constants;

#[cfg(target_pointer_width = "32")]
pub const SIDE_GC_BYTE_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::Global,
    offset: 0,
    log_num_of_bits: 1,
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
};
#[cfg(target_pointer_width = "64")]
pub const SIDE_GC_BYTE_SPEC: SideMetadataSpec = SideMetadataSpec {
    scope: SideMetadataScope::Global,
    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
    log_num_of_bits: 1,
    log_min_obj_size: constants::LOG_BYTES_IN_WORD as usize,
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
fn get_gc_byte<VM: VMBinding>(object: ObjectReference) -> &'static AtomicU8 {
    debug_assert!(VM::VMObjectModel::HAS_GC_BYTE);
    unsafe { &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET).to_ptr::<AtomicU8>() }
}

/// Atomically reads the current value of an object's GC byte.
///
/// Returns an 8-bit unsigned integer
pub fn read_gc_byte<VM: VMBinding>(object: ObjectReference) -> u8 {
    if VM::VMObjectModel::HAS_GC_BYTE {
        get_gc_byte::<VM>(object).load(Ordering::SeqCst)
    } else {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        load_atomic(SIDE_GC_BYTE_SPEC, object.to_address()) as u8
    }
}

/// Atomically writes a new value to the GC byte of an object
pub fn write_gc_byte<VM: VMBinding>(object: ObjectReference, val: u8) {
    if VM::VMObjectModel::HAS_GC_BYTE {
        get_gc_byte::<VM>(object).store(val, Ordering::SeqCst);
    } else {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        store_atomic(SIDE_GC_BYTE_SPEC, object.to_address(), val as usize);
    }
}

/// Atomically performs the compare-and-exchange operation on the GC byte of an object.
///
/// Returns `true` if the operation succeeds.
pub fn compare_exchange_gc_byte<VM: VMBinding>(
    object: ObjectReference,
    old_val: u8,
    new_val: u8,
) -> bool {
    if VM::VMObjectModel::HAS_GC_BYTE {
        get_gc_byte::<VM>(object)
            .compare_exchange(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    } else {
        // is safe, because we only assign SIDE_GCBYTE_ID once
        compare_exchange_atomic(
            SIDE_GC_BYTE_SPEC,
            object.to_address(),
            old_val as usize,
            new_val as usize,
        )
    }
}
