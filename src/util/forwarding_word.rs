/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/utility/ForwardingWord.java
use crate::util::{Address, ObjectReference};
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
const FORWARDING_POINTER_MASK: usize = 0xffff_fff8;

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

pub fn forward_object<VM: VMBinding, CC: CopyContext>(
    object: ObjectReference,
    semantics: AllocationSemantics,
    copy_context: &mut CC,
) -> ObjectReference {
    let new_object = VM::VMObjectModel::copy(object, semantics, copy_context);
    write_forwarding_pointer::<VM>(object, new_object);
    new_object
}

pub fn set_forwarding_pointer<VM: VMBinding>(object: ObjectReference, ptr: ObjectReference) {
    write_forwarding_pointer::<VM>(object, ptr)
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

pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::store_metadata(
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
        object,
        0,
        None,
        Some(Ordering::SeqCst),
    )
}

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
