/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use ::util::{Address, ObjectReference};
use ::vm::ObjectModel;
use ::vm::VMObjectModel;

use ::plan::Allocator;

// ...00
const FORWARDING_NOT_TRIGGERED_YET: u8 = 0;
// ...10
const BEING_FORWARDED: u8 = 2;
// ...11
const FORWARDED: u8 = 3;
// ...11
const FORWARDING_MASK: u8 = 3;
const FORWARDING_BITS: usize = 2;

pub fn attempt_to_forward(object: ObjectReference) -> usize {
    let mut old_value: usize = 0;
    old_value = VMObjectModel::prepare_available_bits(object);
    if (old_value as u8) & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !VMObjectModel::attempt_available_bits(object, old_value, old_value | BEING_FORWARDED as usize) {
        old_value = VMObjectModel::prepare_available_bits(object);
        if (old_value as u8) & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
            return old_value;
        }
    }
    return old_value;
}

pub fn spin_and_get_forwarded_object(object: ObjectReference, status_word: usize) -> ObjectReference {
    let mut status_word = status_word;
    while (status_word as u8) & FORWARDING_MASK == BEING_FORWARDED {
        status_word = VMObjectModel::read_available_bits_word(object);
    }
    if (status_word as u8) & FORWARDING_MASK == FORWARDED {
        unsafe { Address::from_usize(status_word & (!FORWARDING_MASK) as usize).to_object_reference() }
    } else { object }
}

pub fn forward_object(object: ObjectReference, allocator: Allocator) -> ObjectReference {
    let new_object = VMObjectModel::copy(object, allocator);
    VMObjectModel::write_available_bits_word(object, new_object.to_address().as_usize() & FORWARDED as usize);
    new_object
}

pub fn set_forwarding_pointer(object: ObjectReference, ptr: ObjectReference) {
    VMObjectModel::write_available_bits_word(object, ptr.to_address().as_usize() | FORWARDED as usize);
}

pub fn is_forwarded(object: ObjectReference) -> bool {
    VMObjectModel::read_available_byte(object) & FORWARDING_MASK == FORWARDED
}

pub fn is_forwarded_or_being_forwarded(object: ObjectReference) -> bool {
    VMObjectModel::read_available_byte(object) & FORWARDING_MASK != 0
}

pub fn state_is_forwarded_or_being_forwarded(header: usize) -> bool {
    header as u8 & FORWARDING_MASK != 0
}

pub fn state_is_being_forwarded(header: usize) -> bool {
    header as u8 & FORWARDING_MASK == BEING_FORWARDED
}

pub fn clear_forwarding_bits(object: ObjectReference) {
    VMObjectModel::write_available_byte(object, (VMObjectModel::read_available_byte(object) as u8) & !FORWARDING_MASK)
}

pub fn extract_forwarding_pointer(forwarding_word: usize) -> ObjectReference {
    unsafe { Address::from_usize(forwarding_word & (!FORWARDING_MASK as usize)).to_object_reference() }
}