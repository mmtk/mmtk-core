/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use ::util::{Address, ObjectReference};
use ::vm::ObjectModel;

// ...00
const FORWARDING_NOT_TRIGGERED_YET: u8 = 0;
// ...10
const BEING_FORWARDED: u8 = 2;
// ...11
const FORWARDED: u8 = 3;
// ...11
const FORWARDING_MASK: u8 = 3;
const FORWARDING_BITS: usize = 2;

fn attempt_to_forward<T: ObjectModel>(object: ObjectReference) -> usize {
    let mut old_value: usize = 0;
    old_value = T::prepare_available_bits(object);
    if old_value & FORWARDING_MASK as u8 != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !T::attempt_available_bits(object, old_value, old_value | BEING_FORWARDED as usize) {
        old_value = T::prepare_available_bits(object);
        if old_value & FORWARDING_MASK as u8 != FORWARDING_NOT_TRIGGERED_YET {
            return old_value;
        }
    }
    return old_value;
}

fn spin_and_get_forward_object<T: ObjectModel>(object: ObjectReference, status_word: usize) {
    let mut status_word = status_word;
    while status_word & FORWARDING_MASK as usize == BEING_FORWARDED {
        status_word = T::read_available_bits_word(object);
    }
    if status_word as u8 & FORWARDING_MASK == FORWARDED {
        unsafe { Address::from_usize(status_word & (!FORWARDING_MASK) as usize).to_object_reference() }
    } else { object }
}