use crate::util::gc_byte;
/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{constants, Address, ObjectReference};
use crate::vm::ObjectModel;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::plan::{AllocationSemantics, CopyContext};
use crate::vm::VMBinding;

// ...00
const FORWARDING_NOT_TRIGGERED_YET: u8 = 0;
// ...10
const BEING_FORWARDED: u8 = 2;
// ...11
const FORWARDED: u8 = 3;
// ...11
const FORWARDING_MASK: u8 = 3;
#[allow(unused)]
const FORWARDING_BITS: usize = 2;

pub fn attempt_to_forward<VM: VMBinding>(object: ObjectReference) -> u8 {
    let mut old_value = gc_byte::read_gc_byte::<VM>(object);
    if old_value & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !gc_byte::compare_exchange_gc_byte::<VM>(object, old_value, old_value | BEING_FORWARDED) {
        old_value = gc_byte::read_gc_byte::<VM>(object);
        if old_value & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
            return old_value;
        }
    }
    old_value
}

pub fn spin_and_get_forwarded_object<VM: VMBinding>(
    object: ObjectReference,
    gc_byte: u8,
) -> ObjectReference {
    let mut gc_byte = gc_byte;
    while gc_byte & FORWARDING_MASK == BEING_FORWARDED {
        gc_byte = gc_byte::read_gc_byte::<VM>(object);
    }
    if gc_byte & FORWARDING_MASK == FORWARDED {
        let status_word = read_forwarding_word::<VM>(object);
        unsafe {
            match gc_byte_offset_in_forwarding_word::<VM>() {
                Some(fw_offset) => {
                    // fw_offset is 0 for JikesRVM and 56 for OpenJDK
                    Address::from_usize(
                        status_word
                            & !((FORWARDING_MASK as usize)
                                << (-fw_offset * constants::BITS_IN_BYTE as isize)),
                    )
                    .to_object_reference()
                }
                None => Address::from_usize(status_word).to_object_reference(),
            }
        }
    } else {
        panic!(
            "Invalid header value 0x{:x} 0x{:x}",
            gc_byte,
            read_forwarding_word::<VM>(object)
        )
    }
}

pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    semantics: AllocationSemantics,
    copy_context: &mut CC,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    match gc_byte_offset_in_forwarding_word::<VM>() {
        Some(fw_offset) => {
            write_forwarding_word::<VM>(
                object,
                new_object.to_address().as_usize()
                    | (FORWARDED as usize) << (-fw_offset * constants::BITS_IN_BYTE as isize),
            );
        }
        None => {
            gc_byte::write_gc_byte::<VM>(object, FORWARDED);
            write_forwarding_word::<VM>(object, new_object.to_address().as_usize());
        }
    };
    new_object
}

pub fn set_forwarding_pointer<VM: VMBinding>(object: ObjectReference, ptr: ObjectReference) {
    match gc_byte_offset_in_forwarding_word::<VM>() {
        Some(fw_offset) => {
            write_forwarding_word::<VM>(
                object,
                ptr.to_address().as_usize()
                    | (FORWARDED as usize) << (-fw_offset * constants::BITS_IN_BYTE as isize),
            );
        }
        None => {
            gc_byte::write_gc_byte::<VM>(object, FORWARDED);
            write_forwarding_word::<VM>(object, ptr.to_address().as_usize());
        }
    }
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    gc_byte::read_gc_byte::<VM>(object) & FORWARDING_MASK == FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    gc_byte::read_gc_byte::<VM>(object) & FORWARDING_MASK != 0
}

pub fn state_is_forwarded_or_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK != 0
}

pub fn state_is_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK == BEING_FORWARDED
}

pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    gc_byte::write_gc_byte::<VM>(
        object,
        gc_byte::read_gc_byte::<VM>(object) & !FORWARDING_MASK,
    );
}

/// Returns the address of the forwarding word of an object.
///
/// First, depending on the `GC_BYTE_OFFSET` specified by the client VM, MMTk tries to
///     use the word that contains the GC byte, as the forwarding word.
///
/// If the first step is not successful, MMTk chooses the word immediately before or after
///     the word that contains the GC byte.
///
/// Considering the minimum object storage of 2 words, the seconds step always succeeds.
///
fn get_forwarding_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
    match gc_byte_offset_in_forwarding_word::<VM>() {
        // forwarding word is located in the same word as gc byte
        Some(fw_offset) => object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET + fw_offset,
        None => {
            let obj_lowest_addr = VM::VMObjectModel::object_start_ref(object);
            if VM::VMObjectModel::HAS_GC_BYTE {
                let abs_gc_byte_offset = (object.to_address() - obj_lowest_addr) as isize
                    + VM::VMObjectModel::GC_BYTE_OFFSET;
                // e.g. there is more than 8 bytes from lowest object address to gc byte
                if abs_gc_byte_offset >= constants::BYTES_IN_ADDRESS as isize {
                    obj_lowest_addr // forwarding word at the lowest address of the object storage
                } else {
                    obj_lowest_addr + constants::BYTES_IN_ADDRESS // forwarding word at the first word after the lowest address of the object storage
                }
            } else {
                obj_lowest_addr // forwarding word at the lowest address of the object storage
            }
        }
    }
}

pub fn read_forwarding_word<VM: VMBinding>(object: ObjectReference) -> usize {
    unsafe {
        get_forwarding_word_address::<VM>(object).atomic_load::<AtomicUsize>(Ordering::SeqCst)
    }
}

pub fn write_forwarding_word<VM: VMBinding>(object: ObjectReference, val: usize) {
    trace!("GCForwardingWord::write({:#?}, {:x})\n", object, val);
    unsafe {
        get_forwarding_word_address::<VM>(object).atomic_store::<AtomicUsize>(val, Ordering::SeqCst)
    }
}

pub fn compare_exchange_forwarding_word<VM: VMBinding>(
    object: ObjectReference,
    old: usize,
    new: usize,
) -> bool {
    // TODO(Javad): check whether this atomic operation is too strong
    let res = unsafe {
        get_forwarding_word_address::<VM>(object)
            .compare_exchange::<AtomicUsize>(old, new, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    };
    trace!(
        "\nGCForwardingWord::compare_exchange({:#?}, old = {:x}, new = {:x}) -> {}\n",
        object,
        old,
        new,
        res
    );
    res
}

// (This function is only used internal to the `util` module)
//
// This function checks whether the forwarding word and GC byte can be unified (= the forwarding word fits in the word that contains the GC byte).
//
// Returns `None` if the forwarding word and GC byte can not be unified.
// Otherwise, returns `Some(fw_offset)`, where `fw_offset` is the offset of the forwarding word relative to `GC_BYTE_OFFSET`.
//
// A return value of `Some(fw_offset)` implies that GC byte and forwarding word can be loaded/stored with a single instruction.
//
#[cfg(target_endian = "little")]
pub(super) fn gc_byte_offset_in_forwarding_word<VM: VMBinding>() -> Option<isize> {
    let gcbyte_lshift = VM::VMObjectModel::GC_BYTE_OFFSET % constants::BYTES_IN_WORD as isize;
    if VM::VMObjectModel::HAS_GC_BYTE {
        if gcbyte_lshift == 0 {
            // e.g. JikesRVM
            Some(0)
        } else if gcbyte_lshift == (constants::BYTES_IN_WORD - 1) as isize {
            // e.g. OpenJDK
            Some(1 - constants::BYTES_IN_WORD as isize)
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(target_endian = "big")]
pub(super) fn gc_byte_offset_in_forwarding_word<VM: VMBinding>() -> Option<isize> {
    unimplemented!()
}

#[cfg(debug_assertions)]
pub(crate) fn check_alloc_size<VM: VMBinding>(size: usize) {
    debug_assert!(
        if !VM::VMObjectModel::HAS_GC_BYTE || gc_byte_offset_in_forwarding_word::<VM>().is_some() {
            // If there is no gc byte, the min object size is 1 word. We save forwarding pointer in the word.
            // If the gc byte is low/high order byte, the min object size is 1 word. We save forwarding pointer
            // in the word that contains the gc byte.
            size >= constants::BYTES_IN_WORD
        } else {
            // For none of the above cases, the min object size is 2 word. We save forwarding pointer in the next word that does not contain the gc byte.
            size >= 2 * constants::BYTES_IN_WORD
        },
        "allocation size (0x{:x}) is too small!",
        size
    );
}
