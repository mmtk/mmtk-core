use crate::util::object_gc_stats::{
    unifiable_gcbyte_forwarding_word_offset, GCByte, GCForwardingWord,
};
/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{constants, Address, ObjectReference};
use crate::vm::ObjectModel;

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
    let mut old_value = GCByte::read::<VM>(object);
    if old_value & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !GCByte::compare_exchange::<VM>(object, old_value, old_value | BEING_FORWARDED) {
        old_value = GCByte::read::<VM>(object);
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
        gc_byte = GCByte::read::<VM>(object);
    }
    if gc_byte & FORWARDING_MASK == FORWARDED {
        let status_word = GCForwardingWord::read::<VM>(object);
        unsafe {
            match unifiable_gcbyte_forwarding_word_offset::<VM>() {
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
            GCForwardingWord::read::<VM>(object)
        )
    }
}

pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    semantics: AllocationSemantics,
    copy_context: &mut CC,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    match unifiable_gcbyte_forwarding_word_offset::<VM>() {
        Some(fw_offset) => {
            GCForwardingWord::write::<VM>(
                object,
                new_object.to_address().as_usize()
                    | (FORWARDED as usize) << (-fw_offset * constants::BITS_IN_BYTE as isize),
            );
        }
        None => {
            GCByte::write::<VM>(object, FORWARDED);
            GCForwardingWord::write::<VM>(object, new_object.to_address().as_usize());
        }
    };
    new_object
}

pub fn set_forwarding_pointer<VM: VMBinding>(object: ObjectReference, ptr: ObjectReference) {
    match unifiable_gcbyte_forwarding_word_offset::<VM>() {
        Some(fw_offset) => {
            GCForwardingWord::write::<VM>(
                object,
                ptr.to_address().as_usize()
                    | (FORWARDED as usize) << (-fw_offset * constants::BITS_IN_BYTE as isize),
            );
        }
        None => {
            GCByte::write::<VM>(object, FORWARDED);
            GCForwardingWord::write::<VM>(object, ptr.to_address().as_usize());
        }
    }
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    GCByte::read::<VM>(object) & FORWARDING_MASK == FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    GCByte::read::<VM>(object) & FORWARDING_MASK != 0
}

pub fn state_is_forwarded_or_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK != 0
}

pub fn state_is_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK == BEING_FORWARDED
}

pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    GCByte::write::<VM>(object, GCByte::read::<VM>(object) & !FORWARDING_MASK);
}
