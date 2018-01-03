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
    if (old_value as u8) & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !T::attempt_available_bits(object, old_value, old_value | BEING_FORWARDED as usize) {
        old_value = T::prepare_available_bits(object);
        if (old_value as u8) & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
            return old_value;
        }
    }
    return old_value;
}

fn spin_and_get_forwarded_object<T: ObjectModel>(object: ObjectReference, status_word: usize) -> ObjectReference {
    let mut status_word = status_word;
    while (status_word as u8) & FORWARDING_MASK == BEING_FORWARDED {
        status_word = T::read_available_bits_word(object);
    }
    if (status_word as u8) & FORWARDING_MASK == FORWARDED {
        unsafe { Address::from_usize(status_word & (!FORWARDING_MASK) as usize).to_object_reference() }
    } else { object }
}

fn forward_object<T: ObjectModel>(object: ObjectReference, allocator: usize) -> ObjectReference{
    let new_object = T::copy(object, allocator);
    T::write_available_bits_word(object, new_object.to_address().as_usize() & FORWARDED as usize);
    new_object
}

fn set_forwarding_pointer<T: ObjectModel>(object: ObjectReference, ptr: ObjectReference) {
    T::write_available_bits_word(object, ptr.to_address().as_usize() | FORWARDED as usize);
}

fn is_forwarded<T: ObjectModel>(object: ObjectReference) -> bool {
    T::read_available_byte(object) & FORWARDING_MASK == FORWARDED
}

fn is_forwarded_or_being_forwarded<T: ObjectModel>(object: ObjectReference) -> bool {
    T::read_available_byte(object) & FORWARDING_MASK != 0
}

fn state_is_forwarded_or_being_forwarded<T: ObjectModel>(header: usize) -> bool {
    header as u8 & FORWARDING_MASK != 0
}

fn state_is_being_forwarded<T: ObjectModel>(header: usize) -> bool {
    header as u8 & FORWARDING_MASK == BEING_FORWARDED
}

fn clear_forwarding_bits<T: ObjectModel>(object: ObjectReference) {
    T::write_available_byte(object, (T::read_available_byte(object) as u8) & !FORWARDING_MASK)
}

fn extract_forwarding_pointer<T: ObjectModel>(forwarding_word: usize) -> ObjectReference {
    unsafe { Address::from_usize(forwarding_word & (!FORWARDING_MASK as usize)).to_object_reference() }
}