/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{Address, ObjectReference};
use crate::vm::ObjectModel;
use std::sync::atomic::Ordering;

use crate::plan::{AllocationSemantic, CopyContext};
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
    let gc_byte = VM::VMObjectModel::get_gc_byte(object);
    let mut old_value = gc_byte.load(Ordering::SeqCst);
    if old_value & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while old_value
        != gc_byte.compare_and_swap(old_value, old_value | BEING_FORWARDED, Ordering::SeqCst)
    {
        old_value = gc_byte.load(Ordering::SeqCst);
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
    let gc_byte_slot = VM::VMObjectModel::get_gc_byte(object);
    while gc_byte & FORWARDING_MASK == BEING_FORWARDED {
        gc_byte = gc_byte_slot.load(Ordering::SeqCst);
    }
    if gc_byte & FORWARDING_MASK == FORWARDED {
        let status_word = VM::VMObjectModel::read_available_bits_word(object);
        let a = status_word & !((FORWARDING_MASK as usize) << VM::VMObjectModel::GC_BYTE_OFFSET);
        unsafe { Address::from_usize(a).to_object_reference() }
    } else {
        panic!(
            "Invalid header value 0x{:x} 0x{:x}",
            gc_byte,
            VM::VMObjectModel::read_available_bits_word(object)
        )
    }
}

pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    allocator: AllocationSemantic,
    copy_context: &mut CC,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, allocator, copy_context);
    let forwarded = (FORWARDED as usize) << VM::VMObjectModel::GC_BYTE_OFFSET;
    VM::VMObjectModel::write_available_bits_word(
        object,
        new_object.to_address().as_usize() | forwarded,
    );
    new_object
}

pub fn set_forwarding_pointer<VM: VMBinding>(object: ObjectReference, ptr: ObjectReference) {
    let forwarded = (FORWARDED as usize) << VM::VMObjectModel::GC_BYTE_OFFSET;
    VM::VMObjectModel::write_available_bits_word(object, ptr.to_address().as_usize() | forwarded);
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    VM::VMObjectModel::get_gc_byte(object).load(Ordering::Relaxed) & FORWARDING_MASK == FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    VM::VMObjectModel::get_gc_byte(object).load(Ordering::Relaxed) & FORWARDING_MASK != 0
}

pub fn state_is_forwarded_or_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK != 0
}

pub fn state_is_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK == BEING_FORWARDED
}

pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    let gc_byte = VM::VMObjectModel::get_gc_byte(object);
    gc_byte.store(
        gc_byte.load(Ordering::SeqCst) & !FORWARDING_MASK,
        Ordering::SeqCst,
    );
}

// pub fn extract_forwarding_pointer(forwarding_word: usize) -> ObjectReference {
//     unsafe { Address::from_usize(forwarding_word & (!(FORWARDING_MASK as usize))).to_object_reference() }
// }
