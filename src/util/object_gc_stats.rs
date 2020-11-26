use crate::util::{constants, Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// This struct encapsulates operations on the per-object GC byte (metadata)
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

    /// Atomically reads the current value of an object's GC byte.
    ///
    /// Returns an 8-bit unsigned integer
    pub fn read<VM: VMBinding>(object: ObjectReference) -> u8 {
        Self::get_gc_byte::<VM>(object).load(Ordering::SeqCst)
    }

    /// Atomically writes a new value to the GC byte of an object
    pub fn write<VM: VMBinding>(object: ObjectReference, val: u8) {
        Self::get_gc_byte::<VM>(object).store(val, Ordering::SeqCst);
    }

    /// Atomically performs the compare-and-exchange operation on the GC byte of an object.
    ///
    /// Returns `true` if the operation succeeds.
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

/// This struct encapsulates operations on the forwarding word of objects.
pub struct GCForwardingWord {}

impl GCForwardingWord {
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
    #[cfg(target_endian = "little")]
    fn get_object_status_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
        match unifiable_gcbyte_forwarding_word_offset::<VM>() {
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

    #[cfg(target_endian = "big")]
    fn get_object_status_word_address<VM: VMBinding>(object: ObjectReference) -> Address {
        unimplemented!()
    }

    pub fn read<VM: VMBinding>(object: ObjectReference) -> usize {
        unsafe {
            Self::get_object_status_word_address::<VM>(object)
                .atomic_load::<AtomicUsize>(Ordering::SeqCst)
        }
    }

    pub fn write<VM: VMBinding>(object: ObjectReference, val: usize) {
        trace!("GCForwardingWord::write({:#?}, {:x})\n", object, val);
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
        trace!(
            "\nGCForwardingWord::compare_exchange({:#?}, old = {:x}, new = {:x}) -> {}\n",
            object,
            old,
            new,
            res
        );
        res
    }
}

// (This function is only used internal to the `util` module)
//
// This function checks whether the forwarding word and GC byte can be unified (= the forwarding word fits in the word that contains the GC byte).
//
// Returns `None` if the forwarding word and GC byte can not be unified, or if unifiable,
// returns `Some(fw_offset)`, where `fw_offset` is the offset of the forwarding word relative to `GC_BYTE_OFFSET`.
//
// A return value of `Some(fw_offset)` implies that GC byte and forwarding word can be loaded/stored with a single instruction.
//
pub(super) fn unifiable_gcbyte_forwarding_word_offset<VM: VMBinding>() -> Option<isize> {
    let gcbyte_dealignment = VM::VMObjectModel::GC_BYTE_OFFSET % constants::BYTES_IN_WORD as isize;
    if VM::VMObjectModel::HAS_GC_BYTE {
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
    }
}
