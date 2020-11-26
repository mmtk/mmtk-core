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
                &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET).to_ptr::<AtomicU8>()
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
    #[cfg(target_endian = "little")]
    fn get_object_status_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
        // let res = object.to_address() - 12;
        let res = match unifiable_gcbyte_forwarding_word_offset::<VM>() {
            Some(fw_offset) => object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET + fw_offset,
            None => {
                let obj_lowest_addr = VM::VMObjectModel::object_start_ref(object);
                if VM::VMObjectModel::HAS_GC_BYTE {
                    let abs_gc_byte_offset = (object.to_address() - obj_lowest_addr) as isize
                        + VM::VMObjectModel::GC_BYTE_OFFSET;
                    if abs_gc_byte_offset >= constants::BYTES_IN_ADDRESS as isize {
                        obj_lowest_addr
                    } else {
                        object.to_address() + (VM::VMObjectModel::GC_BYTE_OFFSET + 1)
                    }
                } else {
                    obj_lowest_addr
                }
            }
        };
        // info!("get_object_status_word_address({:#?}) -> {:x}", object, res);
        res
    }

    #[cfg(target_endian = "big")]
    fn get_object_status_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
        unimplemented!()
    }

    pub fn read<VM: VMBinding>(object: ObjectReference) -> usize {
        let res = unsafe {
            Self::get_object_status_word_address::<VM>(object)
                .atomic_load::<AtomicUsize>(Ordering::SeqCst)
        };
        // info!("***GCForwardingWord::read({:#?}) -> {:x}", object, res);
        res
    }

    pub fn write<VM: VMBinding>(object: ObjectReference, val: usize) {
        // info!("***GCForwardingWord::write({:#?}, {:x})\n", object, val);
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

pub(super) fn unifiable_gcbyte_forwarding_word_offset<VM: VMBinding>() -> Option<isize> {
    let gcbyte_dealignment = VM::VMObjectModel::GC_BYTE_OFFSET % constants::BYTES_IN_WORD as isize;
    let res = if VM::VMObjectModel::HAS_GC_BYTE {
        if gcbyte_dealignment == 0 {
            // e.g. JikesRVM
            Some(0)
        } else if gcbyte_dealignment == (constants::BYTES_IN_WORD - 1) as isize {
            // e.g. OpenJDK
            Some(1 - constants::BYTES_IN_WORD as isize)
        } else {
            None
        }
    } else {
        None
    };

    res
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
