use crate::util::copy::*;
use crate::util::metadata::MetadataSpec;
use crate::util::{constants, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::Ordering;

const FORWARDING_NOT_TRIGGERED_YET: u8 = 0b00;
const BEING_FORWARDED: u8 = 0b10;
const FORWARDED: u8 = 0b11;
const FORWARDING_MASK: u8 = 0b11;
#[allow(unused)]
const FORWARDING_BITS: usize = 2;

// copy address mask
#[cfg(target_pointer_width = "64")]
const FORWARDING_POINTER_MASK: usize = 0x00ff_ffff_ffff_fff8;
#[cfg(target_pointer_width = "32")]
const FORWARDING_POINTER_MASK: usize = 0xffff_fffc;

/// Attempt to become the worker thread who will forward the object.
/// The successful worker will set the object forwarding bits to BEING_FORWARDED, preventing other workers from forwarding the same object.
pub fn attempt_to_forward<VM: VMBinding>(object: ObjectReference) -> u8 {
    loop {
        let old_value = get_forwarding_status::<VM>(object);
        if old_value != FORWARDING_NOT_TRIGGERED_YET
            || VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    BEING_FORWARDED,
                    None,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            return old_value;
        }
    }
}

/// Spin-wait for the object's forwarding to become complete and then read the forwarding pointer to the new object.
///
/// # Arguments:
///
/// * `object`: the forwarded/being_forwarded object.
/// * `forwarding_bits`: the last state of the forwarding bits before calling this function.
///
/// Returns a reference to the new object.
///
pub fn spin_and_get_forwarded_object<VM: VMBinding>(
    object: ObjectReference,
    forwarding_bits: u8,
) -> ObjectReference {
    let mut forwarding_bits = forwarding_bits;
    while forwarding_bits == BEING_FORWARDED {
        forwarding_bits = get_forwarding_status::<VM>(object);
    }

    if forwarding_bits == FORWARDED {
        read_forwarding_pointer::<VM>(object)
    } else {
        // For some policies (such as Immix), we can have interleaving such that one thread clears
        // the forwarding word while another thread was stuck spinning in the above loop.
        // See: https://github.com/mmtk/mmtk-core/issues/579
        debug_assert!(
            forwarding_bits == FORWARDING_NOT_TRIGGERED_YET,
            "Invalid/Corrupted forwarding word {:x} for object {}",
            forwarding_bits,
            object,
        );
        object
    }
}

pub fn forward_object<VM: VMBinding>(
    object: ObjectReference,
    semantics: CopySemantics,
    copy_context: &mut GCWorkerCopyContext<VM>,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    if let Some(shift) = forwarding_bits_offset_in_forwarding_pointer::<VM>() {
        VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.store_atomic::<VM, usize>(
            object,
            new_object.to_raw_address().as_usize() | ((FORWARDED as usize) << shift),
            None,
            Ordering::SeqCst,
        )
    } else {
        write_forwarding_pointer::<VM>(object, new_object);
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.store_atomic::<VM, u8>(
            object,
            FORWARDED,
            None,
            Ordering::SeqCst,
        );
    }
    new_object
}

/// Return the forwarding bits for a given `ObjectReference`.
pub fn get_forwarding_status<VM: VMBinding>(object: ObjectReference) -> u8 {
    VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.load_atomic::<VM, u8>(
        object,
        None,
        Ordering::SeqCst,
    )
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) == FORWARDED
}

fn is_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) == BEING_FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) != FORWARDING_NOT_TRIGGERED_YET
}

pub fn state_is_forwarded_or_being_forwarded(forwarding_bits: u8) -> bool {
    forwarding_bits != FORWARDING_NOT_TRIGGERED_YET
}

pub fn state_is_being_forwarded(forwarding_bits: u8) -> bool {
    forwarding_bits == BEING_FORWARDED
}

/// Zero the forwarding bits of an object.
/// This function is used on new objects.
pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.store_atomic::<VM, u8>(
        object,
        0,
        None,
        Ordering::SeqCst,
    )
}

/// Read the forwarding pointer of an object.
/// This function is called on forwarded/being_forwarded objects.
pub fn read_forwarding_pointer<VM: VMBinding>(object: ObjectReference) -> ObjectReference {
    debug_assert!(
        is_forwarded_or_being_forwarded::<VM>(object),
        "read_forwarding_pointer called for object {:?} that has not started forwarding!",
        object,
    );

    // We write the forwarding poiner. We know it is an object reference.
    unsafe {
        ObjectReference::from_raw_address(crate::util::Address::from_usize(
            VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.load_atomic::<VM, usize>(
                object,
                Some(FORWARDING_POINTER_MASK),
                Ordering::SeqCst,
            ),
        ))
    }
}

/// Write the forwarding pointer of an object.
/// This function is called on being_forwarded objects.
pub fn write_forwarding_pointer<VM: VMBinding>(
    object: ObjectReference,
    new_object: ObjectReference,
) {
    debug_assert!(
        is_being_forwarded::<VM>(object),
        "write_forwarding_pointer called for object {:?} that is not being forwarded! Forwarding state = 0x{:x}",
        object,
        get_forwarding_status::<VM>(object),
    );

    trace!("write_forwarding_pointer({}, {})", object, new_object);
    VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.store_atomic::<VM, usize>(
        object,
        new_object.to_raw_address().as_usize(),
        Some(FORWARDING_POINTER_MASK),
        Ordering::SeqCst,
    )
}

/// (This function is only used internal to the `util` module)
///
/// This function checks whether the forwarding pointer and forwarding bits can be written in the same atomic operation.
///
/// Returns `None` if this is not possible.
/// Otherwise, returns `Some(shift)`, where `shift` is the left shift needed on forwarding bits.
///
#[cfg(target_endian = "little")]
pub(super) fn forwarding_bits_offset_in_forwarding_pointer<VM: VMBinding>() -> Option<isize> {
    use std::ops::Deref;
    // if both forwarding bits and forwarding pointer are in-header
    match (
        VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.deref(),
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.deref(),
    ) {
        (MetadataSpec::InHeader(fp), MetadataSpec::InHeader(fb)) => {
            let maybe_shift = fb.bit_offset - fp.bit_offset;
            if maybe_shift >= 0 && maybe_shift < constants::BITS_IN_WORD as isize {
                Some(maybe_shift)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(target_endian = "big")]
pub(super) fn forwarding_bits_offset_in_forwarding_pointer<VM: VMBinding>() -> Option<isize> {
    unimplemented!()
}
