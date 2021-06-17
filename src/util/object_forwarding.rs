/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{constants, Address, ObjectReference};
use crate::vm::ObjectModel;
use std::sync::atomic::Ordering;

use crate::plan::{AllocationSemantics, CopyContext};
use crate::vm::VMBinding;

// ...00
const FORWARDING_NOT_TRIGGERED_YET: usize = 0;
// ...10
const BEING_FORWARDED: usize = 2;
// ...11
const FORWARDED: usize = 3;
// ...11
const FORWARDING_MASK: usize = 3;
#[allow(unused)]
const FORWARDING_BITS: usize = 2;

// copy address mask
#[cfg(target_pointer_width = "64")]
const FORWARDING_POINTER_MASK: usize = 0x00ff_ffff_ffff_fff8;
#[cfg(target_pointer_width = "32")]
const FORWARDING_POINTER_MASK: usize = 0xffff_fffc;

/// Attempt to become the worker thread who will forward the object.
/// The successful worker will set the object forwarding bits to BEING_FORWARDED, preventing other workers from forwarding the same object.
pub fn attempt_to_forward<VM: VMBinding>(object: ObjectReference) -> usize {
    loop {
        let old_value = VM::VMObjectModel::load_metadata(
            VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        if old_value != FORWARDING_NOT_TRIGGERED_YET
            || VM::VMObjectModel::compare_exchange_metadata(
                VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
                object,
                old_value,
                old_value | BEING_FORWARDED,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
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
    forwarding_bits: usize,
) -> ObjectReference {
    let mut forwarding_bits = forwarding_bits;
    while forwarding_bits == BEING_FORWARDED {
        forwarding_bits = VM::VMObjectModel::load_metadata(
            VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
    }
    if forwarding_bits == FORWARDED {
        read_forwarding_pointer::<VM>(object)
    } else {
        panic!(
            "Invalid forwarding state 0x{:x} 0x{:x} for object{}",
            forwarding_bits,
            read_forwarding_pointer::<VM>(object),
            object
        )
    }
}

/// Copy the object, mark it as forwarded and return a reference to the new object (copy).
pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    semantics: AllocationSemantics,
    copy_context: &mut CC,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    if let Some(shift) = forwarding_bits_offset_in_forwarding_pointer::<VM>() {
        VM::VMObjectModel::store_metadata(
            VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
            object,
            new_object.to_address().as_usize() | (FORWARDED << shift),
            None,
            Some(Ordering::SeqCst),
        )
    } else {
        write_forwarding_pointer::<VM>(object, new_object);
        VM::VMObjectModel::store_metadata(
            VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
            object,
            FORWARDED,
            None,
            Some(Ordering::SeqCst),
        );
    }
    new_object
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    VM::VMObjectModel::load_metadata(
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
        object,
        None,
        Some(Ordering::SeqCst),
    ) == FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    VM::VMObjectModel::load_metadata(
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
        object,
        None,
        Some(Ordering::SeqCst),
    ) != FORWARDING_NOT_TRIGGERED_YET
}

pub fn state_is_forwarded_or_being_forwarded(forwarding_bits: usize) -> bool {
    forwarding_bits != FORWARDING_NOT_TRIGGERED_YET
}

pub fn state_is_being_forwarded(forwarding_bits: usize) -> bool {
    forwarding_bits == BEING_FORWARDED
}

/// Zero the forwarding bits of an object.
/// This function is used on new objects.
pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::store_metadata(
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
        object,
        0,
        None,
        Some(Ordering::SeqCst),
    )
}

/// Read the forwarding pointer of an object.
/// This function is called on forwarded/being_forwarded objects.
pub fn read_forwarding_pointer<VM: VMBinding>(object: ObjectReference) -> ObjectReference {
    unsafe {
        Address::from_usize(VM::VMObjectModel::load_metadata(
            VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
            object,
            Some(FORWARDING_POINTER_MASK),
            Some(Ordering::SeqCst),
        ))
        .to_object_reference()
    }
}

/// Write the forwarding pointer of an object.
/// This function is called on being_forwarded objects.
pub fn write_forwarding_pointer<VM: VMBinding>(
    object: ObjectReference,
    new_object: ObjectReference,
) {
    trace!("GCForwardingWord::write({:#?}, {:x})\n", object, new_object);
    VM::VMObjectModel::store_metadata(
        VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
        object,
        new_object.to_address().as_usize(),
        Some(FORWARDING_POINTER_MASK),
        Some(Ordering::SeqCst),
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
    // if both forwarding bits and forwarding pointer are in-header
    if !VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.is_side_metadata
        && !VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.is_side_metadata
    {
        let maybe_shift = VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.offset
            - VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.offset;
        if maybe_shift < constants::BITS_IN_WORD as isize {
            Some(maybe_shift)
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(target_endian = "big")]
pub(super) fn forwarding_bits_offset_in_forwarding_pointer<VM: VMBinding>() -> Option<isize> {
    unimplemented!()
}
