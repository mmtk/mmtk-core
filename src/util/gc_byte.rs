use crate::util::side_metadata::{SideMetadata, SideMetadataID};
use crate::util::{constants, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicU8, Ordering};

/// This struct encapsulates operations on the per-object GC byte (metadata)
pub struct GCByte {}

static SIDE_GCBYTE_ID: SideMetadataID = SideMetadataID::new();

#[allow(clippy::cast_ref_to_mut)]
#[allow(clippy::mut_from_ref)]
pub(crate) unsafe fn init_side_gcbyte() {
    *(&SIDE_GCBYTE_ID as *const SideMetadataID as *mut SideMetadataID) =
        SideMetadata::add_meta_bits(
            // constants::BITS_IN_BYTE,
            2,
            constants::LOG_BYTES_IN_WORD as usize,
        );
}

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
    if VM::VMObjectModel::HAS_GC_BYTE {
        unsafe { &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET).to_ptr::<AtomicU8>() }
    } else {
        // todo!("\"HAS_GC_BYTE == false\" is not supported yet")
        unreachable!()
    }
}

/// Atomically reads the current value of an object's GC byte.
///
/// Returns an 8-bit unsigned integer
pub fn read_gc_byte<VM: VMBinding>(object: ObjectReference) -> u8 {
    if VM::VMObjectModel::HAS_GC_BYTE {
        get_gc_byte::<VM>(object).load(Ordering::SeqCst)
    } else {
        SideMetadata::load_atomic(SIDE_GCBYTE_ID, object.to_address()) as u8
    }
}

/// Atomically writes a new value to the GC byte of an object
pub fn write_gc_byte<VM: VMBinding>(object: ObjectReference, val: u8) {
    if VM::VMObjectModel::HAS_GC_BYTE {
        get_gc_byte::<VM>(object).store(val, Ordering::SeqCst);
    } else {
        SideMetadata::store_atomic(SIDE_GCBYTE_ID, object.to_address(), val as usize);
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
        SideMetadata::compare_exchange_atomic(
            SIDE_GCBYTE_ID,
            object.to_address(),
            old_val as usize,
            new_val as usize,
        )
    }
}
