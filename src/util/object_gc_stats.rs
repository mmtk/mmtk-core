use crate::util::{constants, Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

const STATUS_WORD_OFFSET: usize = std::mem::size_of::<usize>();

pub struct GCByte {}

// TODO: we probably need to add non-atomic versions of the read and write methods
impl GCByte {
    /// Return the GC byte of an object as an atomic.
    ///
    /// MMTk requires *exactly one byte* for each object as per-object metadata.
    ///
    /// If the client VM provides the one byte in its object headers
    /// (see [trait ObjectModel](crate::vm::ObjectModel)),
    /// MMTk uses that byte as the per-object metadata.
    /// Otherwise, MMTk provides the metadata on its side.
    ///
    fn get_gc_byte<VM: VMBinding>(object: ObjectReference) -> &'static AtomicU8 {
        if VM::VMObjectModel::HAS_GC_BYTE {
            unsafe {
                &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET / 8).to_ptr::<AtomicU8>()
            }
        } else {
            todo!("\"HAS_GC_BYTE == false\" is not supported yet")
        }
    }

    pub fn read<VM: VMBinding>(object: ObjectReference) -> u8 {
        Self::get_gc_byte::<VM>(object).load(Ordering::SeqCst)
    }

    pub fn write<VM: VMBinding>(object: ObjectReference, val: u8) {
        Self::get_gc_byte::<VM>(object).store(val, Ordering::SeqCst);
    }

    pub fn compare_exchange<VM: VMBinding>(
        object: ObjectReference,
        old_val: u8,
        new_val: u8,
    ) -> bool {
        Self::get_gc_byte::<VM>(object)
            .compare_exchange(old_val, new_val, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

pub struct GCForwardingWord {}

impl GCForwardingWord {
    fn get_object_status_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
        // let res = VM::VMObjectModel::object_start_ref(object) + STATUS_WORD_OFFSET;
        let res = object.to_address()
            + ((VM::VMObjectModel::GC_BYTE_OFFSET / constants::BITS_IN_ADDRESS as isize + 1)
                * constants::BITS_IN_ADDRESS as isize
                / 8);
        // debug!("get_object_status_word_address({:#?}) -> {:x}", object, res);
        res
    }

    pub fn read<VM: VMBinding>(object: ObjectReference) -> usize {
        let res = unsafe {
            Self::get_object_status_word_address::<VM>(object)
                .atomic_load::<AtomicUsize>(Ordering::SeqCst)
        };
        info!("***GCForwardingWord::read({:#?}) -> {:x}", object, res);
        res
    }

    pub fn write<VM: VMBinding>(object: ObjectReference, val: usize) {
        info!("***GCForwardingWord::write({:#?}, {:x})\n", object, val);
        unsafe {
            Self::get_object_status_word_address::<VM>(object)
                .atomic_store::<AtomicUsize>(val, Ordering::SeqCst)
        }
    }

    pub fn compare_exchange<VM: VMBinding>(
        object: ObjectReference,
        old: usize,
        new: usize,
    ) -> bool {
        // TODO(Javad): check whether this atomic operation is too strong
        let res = unsafe {
            Self::get_object_status_word_address::<VM>(object)
                .compare_exchange::<AtomicUsize>(old, new, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        };
        info!(
            "\nGCForwardingWord::compare_exchange({:#?}, old = {:x}, new = {:x}) -> {}\n",
            object, old, new, res
        );
        res
    }
}

// fn get_object_status_word_address(object: ObjectReference) -> Address {
//     let res = object.to_address() + STATUS_WORD_OFFSET;
//     debug!("get_object_status_word_address({:#?}) -> {:x}", object, res);
//     res
// }

// pub fn read_object_status_word(object: ObjectReference) -> usize {
//     let res = unsafe {
//         get_object_status_word_address(object).atomic_load::<AtomicUsize>(Ordering::SeqCst)
//     };
//     debug!("read_object_status_word({:#?}) -> {:x}", object, res);
//     res
// }

// pub fn write_object_status_word(object: ObjectReference, val: usize) {
//     debug!("write_object_status_word({:#?}, {:x})", object, val);
//     unsafe {
//         get_object_status_word_address(object).atomic_store::<AtomicUsize>(val, Ordering::SeqCst)
//     }
// }

// pub fn compare_exchange_object_status_word(
//     object: ObjectReference,
//     old: usize,
//     new: usize,
// ) -> bool {
//     // TODO(Javad): check whether this atomic operation is too strong
//     let res = unsafe {
//         get_object_status_word_address(object)
//             .compare_exchange::<AtomicUsize>(old, new, Ordering::SeqCst, Ordering::SeqCst)
//             .is_ok()
//     };
//     debug!(
//         "compare_exchange_object_status_word({:#?}, old = {:x}, new = {:x}) -> {}",
//         object, old, new, res
//     );
//     res
// }
