/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{object_gc_stats, Address, ObjectReference};
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
    let mut old_value = object_gc_stats::GCByte::read::<VM>(object);
    if old_value & FORWARDING_MASK != FORWARDING_NOT_TRIGGERED_YET {
        return old_value;
    }
    while !object_gc_stats::GCByte::compare_exchange::<VM>(
        object,
        old_value,
        old_value | BEING_FORWARDED,
    ) {
        old_value = object_gc_stats::GCByte::read::<VM>(object);
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
        gc_byte = object_gc_stats::GCByte::read::<VM>(object);
    }
    if gc_byte & FORWARDING_MASK == FORWARDED {
        let status_word = object_gc_stats::GCForwardingWord::read::<VM>(object);
        let res = unsafe { Address::from_usize(status_word).to_object_reference() };
        info!(
            "**spin_and_get_forwarded_object({:?},{:?}) -> {:?}",
            object, gc_byte, res
        );
        return res;
    } else {
        panic!(
            "Invalid header value 0x{:x} 0x{:x}",
            gc_byte,
            object_gc_stats::GCForwardingWord::read::<VM>(object)
        )
    }
}

pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    semantics: AllocationSemantics,
    copy_context: &mut CC,
) -> ObjectReference {
    info!("**forward_object({:?})", object);
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    object_gc_stats::GCByte::write::<VM>(object, FORWARDED);
    object_gc_stats::GCForwardingWord::write::<VM>(object, new_object.to_address().as_usize());
    new_object
}

pub fn set_forwarding_pointer<VM: VMBinding>(object: ObjectReference, ptr: ObjectReference) {
    object_gc_stats::GCByte::write::<VM>(object, FORWARDED);
    object_gc_stats::GCForwardingWord::write::<VM>(object, ptr.to_address().as_usize());
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    object_gc_stats::GCByte::read::<VM>(object) & FORWARDING_MASK == FORWARDED
}

pub fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    object_gc_stats::GCByte::read::<VM>(object) & FORWARDING_MASK != 0
}

pub fn state_is_forwarded_or_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK != 0
}

pub fn state_is_being_forwarded(gc_byte: u8) -> bool {
    gc_byte & FORWARDING_MASK == BEING_FORWARDED
}

pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    object_gc_stats::GCByte::write::<VM>(
        object,
        object_gc_stats::GCByte::read::<VM>(object) & !FORWARDING_MASK,
    );
}
